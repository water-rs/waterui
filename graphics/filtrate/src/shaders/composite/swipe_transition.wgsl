// Swipe transition: reveals the target image (`aux0`) along `direction`
// (0 left to right, 1 right to left, 2 top to bottom, 3 bottom to top) as
// `progress` goes from 0 to 1, with a `softness`-wide feathered edge.

struct Params {
    progress: f32,
    softness: f32,
    direction: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, aux0: texture_2d<f32>) -> vec4<f32> {
    let base = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let progress = clamp(params.progress, 0.0, 1.0);
    let softness = max(params.softness, 0.001);
    var edge = uv.x;
    switch u32(params.direction + 0.5) {
        case 1u: {
            edge = 1.0 - uv.x;
        }
        case 2u: {
            edge = uv.y;
        }
        case 3u: {
            edge = 1.0 - uv.y;
        }
        default: {}
    }
    let reveal = smoothstep(progress - softness, progress + softness, edge);
    return mix(base, texel_at(aux0, uv), reveal);
}
