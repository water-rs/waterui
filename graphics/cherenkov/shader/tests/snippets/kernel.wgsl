struct Params {
    step: vec2<f32>,
}

// Declares `input_point_sampler`: the executor must bind a nearest sampler,
// so every sample is an exact texel read and a colour prefix can fold in.
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let left = textureSampleLevel(input, input_point_sampler, uv - params.step, 0.0);
    let centre = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let right = textureSampleLevel(input, input_point_sampler, uv + params.step, 0.0);
    return (left + 2.0 * centre + right) * 0.25;
}
