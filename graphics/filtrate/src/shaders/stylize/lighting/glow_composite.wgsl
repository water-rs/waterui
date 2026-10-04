// Bloom and gloom, second pass: finishes the separable box blur of the
// highlight energy vertically and composites it onto the original — the
// input of the first pass, bound as `aux0`. `polarity` is 1 for bloom, which
// adds the glow (valid premultiplied compositing, alpha unchanged), and -1
// for gloom, which subtracts it and clamps at zero.

struct Params {
    radius: f32,
    intensity: f32,
    polarity: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let pixel = floor(uv * size);
    let radius = max(i32(round(params.radius)), 1);
    let intensity = max(params.intensity, 0.0);

    var sum = vec3<f32>(0.0);
    for (var y = -radius; y <= radius; y++) {
        sum += load(input, input_point_sampler, size, pixel + vec2<f32>(0.0, f32(y))).rgb;
    }
    let glow = sum / f32(2 * radius + 1);

    let base = texel_at(aux0, uv);
    let shifted = base.rgb + params.polarity * glow * intensity;
    let rgb = select(shifted, max(shifted, vec3<f32>(0.0)), params.polarity < 0.0);
    return vec4<f32>(rgb, base.a);
}
