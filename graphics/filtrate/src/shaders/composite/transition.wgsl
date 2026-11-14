// Transition: reveals the target image (`aux0`) left to right as `progress`
// goes from 0 to 1, with a `softness`-wide feathered edge.

struct Params {
    progress: f32,
    softness: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let progress = clamp(params.progress, 0.0, 1.0);
    let softness = max(params.softness, 0.001);
    let edge = smoothstep(progress - softness, progress + softness, uv.x);
    return mix(base, texel_at(aux0, uv), edge);
}
