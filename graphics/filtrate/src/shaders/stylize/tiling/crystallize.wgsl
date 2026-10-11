// Crystallize: Voronoi cells of jittered grid seed points, each cell
// flat-coloured by the input at its seed. Membership is decided per pixel
// against the 3x3 neighbouring seeds, which produces irregular polygonal
// facets.
//
// Parameters: cell size in pixels.

struct Params {
    cell: f32,
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let cell = max(params.cell, 1.0);
    let pixel = floor(uv * size) + vec2<f32>(0.5);
    let grid = floor(pixel / cell);

    var best_dist = 1e30;
    var best_seed = pixel;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let neighbor = grid + vec2<f32>(f32(dx), f32(dy));
            let jitter = (hash22(neighbor) - vec2<f32>(0.5)) * 0.8;
            let seed = (neighbor + vec2<f32>(0.5) + jitter) * cell;
            let d = distance(pixel, seed);
            if d < best_dist {
                best_dist = d;
                best_seed = seed;
            }
        }
    }
    return textureSampleLevel(input, input_point_sampler, best_seed / size, 0.0);
}
