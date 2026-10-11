// Sharpen: adds the laplacian detail of the four direct neighbours, scaled
// by `amount`. Alpha keeps the centre's coverage.

struct Params {
    amount: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let pixel = floor(uv * size);

    let center = load(input, input_point_sampler, size, pixel);
    let top = load(input, input_point_sampler, size, pixel + vec2<f32>(0.0, -1.0));
    let bottom = load(input, input_point_sampler, size, pixel + vec2<f32>(0.0, 1.0));
    let left = load(input, input_point_sampler, size, pixel + vec2<f32>(-1.0, 0.0));
    let right = load(input, input_point_sampler, size, pixel + vec2<f32>(1.0, 0.0));

    let laplacian = center * 4.0 - top - bottom - left - right;
    let result = center + laplacian * params.amount;
    return vec4<f32>(result.rgb, center.a);
}
