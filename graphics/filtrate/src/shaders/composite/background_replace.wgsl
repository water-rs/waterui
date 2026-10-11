// Background replace: composites the input over a replacement background
// (`aux1`) through a foreground matte (`aux0`, red channel) with a
// `edge_softness`-wide feathered edge.

struct Params {
    edge_softness: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>, aux1: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let edge_softness = max(params.edge_softness, 0.0001);
    let matte = clamp(texel_at(aux0, uv).r, 0.0, 1.0);
    let foreground = smoothstep(0.5 - edge_softness, 0.5 + edge_softness, matte);
    return mix(texel_at(aux1, uv), base, foreground);
}
