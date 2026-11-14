// Temporal denoise: mixes the input with the previous frame (`aux0`),
// reprojected by the motion vectors (`aux1`, red and green mapped from
// [0, 1] to [-1, 1], in pixels), weighted by `history_weight`.

struct Params {
    history_weight: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>, aux1: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let history_weight = clamp(params.history_weight, 0.0, 0.99);
    let motion = texel_at(aux1, uv).rg * 2.0 - vec2<f32>(1.0);
    let history = texel_at(aux0, uv - motion / size);
    return mix(base, history, history_weight);
}
