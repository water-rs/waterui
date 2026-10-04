// Bloom and gloom, first pass: extracts thresholded highlight energy and
// box-accumulates it horizontally.
//
// Each tap contributes `color * max(luma - threshold, 0)` and the pass
// normalizes by tap count, so the threshold controls the magnitude of the
// glow — a fully sub-threshold neighbourhood contributes exactly zero.
// (Normalizing by summed weights instead would cancel the threshold.)

struct Params {
    radius: f32,
    threshold: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let pixel = floor(uv * size);
    let radius = max(i32(round(params.radius)), 1);
    let threshold = max(params.threshold, 0.0);

    var sum = vec3<f32>(0.0);
    for (var x = -radius; x <= radius; x++) {
        let texel = load(input, input_point_sampler, size, pixel + vec2<f32>(f32(x), 0.0));
        sum += texel.rgb * max(dot(texel.rgb, space.luma) - threshold, 0.0);
    }
    return vec4<f32>(sum / f32(2 * radius + 1), 0.0);
}
