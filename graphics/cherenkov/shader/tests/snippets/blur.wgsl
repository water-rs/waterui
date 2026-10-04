struct Params {
    step: vec2<f32>,
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let left = textureSampleLevel(input, input_sampler, uv - params.step, 0.0);
    let centre = textureSampleLevel(input, input_sampler, uv, 0.0);
    let right = textureSampleLevel(input, input_sampler, uv + params.step, 0.0);
    return (left + 2.0 * centre + right) * 0.25;
}
