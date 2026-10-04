struct Params {
    amount: f32,
}

// `size` is the stage input's extent in pixels, supplied by the executor:
// the stage reads it, and a colour prefix still folds in.
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let texel = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    return texel * params.amount / (size.x * size.y);
}
