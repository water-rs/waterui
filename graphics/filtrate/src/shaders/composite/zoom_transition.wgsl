// Zoom transition: cross-fades to the target image (`aux0`) while the input
// zooms out of, and the target into, a centre point.

struct Params {
    progress: f32,
    amount: f32,
    center_x: f32,
    center_y: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let progress = clamp(params.progress, 0.0, 1.0);
    let amount = max(params.amount, 0.0);
    let to_center = vec2<f32>(params.center_x, params.center_y) - uv;
    let source_uv = uv - to_center * amount * (1.0 - progress);
    let target_uv = uv + to_center * amount * progress;
    let source = textureSampleLevel(input, input_point_sampler, source_uv, 0.0);
    return mix(source, texel_at(aux0, target_uv), progress);
}
