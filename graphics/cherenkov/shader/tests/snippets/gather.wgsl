struct Params {
    amount: f32,
}

// `textureGather` makes folding non-equivalent even through a point sampler:
// the prefix would apply to a gathered component vector.
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let gathered = textureGather(0, input, input_point_sampler, uv);
    let sum = gathered.x + gathered.y + gathered.z + gathered.w;
    return vec4<f32>(sum * params.amount, 0.0, 0.0, 1.0);
}
