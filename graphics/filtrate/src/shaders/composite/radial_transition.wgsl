// Radial transition: reveals the target image (`aux0`) outward from a
// centre point as `progress` goes from 0 to 1, with a `softness`-wide
// feathered edge.

struct Params {
    progress: f32,
    softness: f32,
    center_x: f32,
    center_y: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let progress = clamp(params.progress, 0.0, 1.0);
    let softness = max(params.softness, 0.001);
    let radius = distance(uv, vec2<f32>(params.center_x, params.center_y));
    let edge = smoothstep(progress - softness, progress + softness, radius * 1.41421356);
    return mix(base, texel_at(aux0, uv), edge);
}
