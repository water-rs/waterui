// Pixellate: every pixel takes the texel at the centre of its square cell.
// Nearest sampling is intentional — pixellate wants hard-edged cells.
//
// Parameters: cell size in pixels.

struct Params {
    size: f32,
}

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let cell = max(params.size, 1.0);
    let pixel = floor(uv * size) + vec2<f32>(0.5);
    let cell_center = floor(pixel / cell) * cell + vec2<f32>(cell * 0.5);
    return textureSampleLevel(input, input_point_sampler, cell_center / size, 0.0);
}
