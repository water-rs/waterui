// Displacement transition: cross-fades to the target image (`aux0`) while
// the displacement map (`aux1`, red and green mapped from [0, 1] to
// [-1, 1]) pushes the input out and pulls the target in, `scale` pixels at
// full strength.

struct Params {
    progress: f32,
    scale: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>, aux1: texture_2d<f32>) -> vec4<f32> {
    let progress = clamp(params.progress, 0.0, 1.0);
    let scale = max(params.scale, 0.0);
    let displacement = texel_at(aux1, uv).rg * 2.0 - vec2<f32>(1.0);
    let source_uv = uv - displacement * scale * progress / size;
    let target_uv = uv + displacement * scale * (1.0 - progress) / size;
    let source = textureSampleLevel(input, input_point_sampler, source_uv, 0.0);
    return mix(source, texel_at(aux0, target_uv), progress);
}
