// Zoom blur: a radial streak toward a focal point, in twelve bilinearly
// filtered taps weighted toward the pixel itself.

struct Params {
    amount: f32,
    center_x: f32,
    center_y: f32,
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let amount = max(params.amount, 0.0);
    if amount <= 0.0001 {
        return textureSampleLevel(input, input_sampler, (floor(uv * size) + 0.5) / size, 0.0);
    }

    let direction = vec2<f32>(params.center_x, params.center_y) - uv;
    let samples: i32 = 12;
    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;
    for (var i = 0; i < samples; i++) {
        let t = f32(i) / f32(samples - 1);
        let weight = 1.0 - t * 0.65;
        sum += textureSampleLevel(input, input_sampler, uv + direction * amount * t, 0.0) * weight;
        total_weight += weight;
    }
    return sum / total_weight;
}
