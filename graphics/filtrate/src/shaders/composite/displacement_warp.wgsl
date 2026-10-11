// Displacement warp: offsets each pixel's sample by the displacement map
// (`aux0`, red and green mapped from [0, 1] to [-1, 1]) times `scale`
// pixels.

struct Params {
    scale_x: f32,
    scale_y: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let displacement = texel_at(aux0, uv).rg * 2.0 - vec2<f32>(1.0);
    let warped_uv = uv + displacement * vec2<f32>(params.scale_x, params.scale_y) / size;
    return textureSampleLevel(input, input_point_sampler, warped_uv, 0.0);
}
