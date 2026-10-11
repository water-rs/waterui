// Mirror tile: repeats the image with alternate tiles mirrored, so tile
// seams are continuous. Even-indexed tiles (including the first) pass
// through unmirrored; `repeat = 1` reproduces the input.

struct Params {
    repeat_x: f32,
    repeat_y: f32,
}

fn mirror_repeat(v: f32) -> f32 {
    let tiled = fract(v);
    let odd_tile = (i32(floor(v)) & 1) != 0;
    return select(tiled, 1.0 - tiled, odd_tile);
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let tiled_uv = vec2<f32>(
        mirror_repeat(uv.x * max(params.repeat_x, 1.0)),
        mirror_repeat(uv.y * max(params.repeat_y, 1.0)),
    );
    return textureSampleLevel(input, input_sampler, tiled_uv, 0.0);
}
