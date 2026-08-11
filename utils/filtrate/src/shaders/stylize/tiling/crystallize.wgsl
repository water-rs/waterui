// Crystallize: Voronoi cells of jittered grid seed points, each cell
// flat-colored by the source at its seed. Membership is decided per pixel
// against the 3x3 neighbouring seeds, which is what produces irregular
// polygonal facets (jittering only the sample point inside fixed square
// cells would just be noisy pixellation).
//
// Parameters: cell size in output pixels.

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let cell = max(param(0u), 1.0);
    let pixel = vec2<f32>(gid.xy) + vec2<f32>(0.5);
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

    let seed_uv = best_seed / uniforms.output_dimensions;
    let coord = vec2<i32>(seed_uv * uniforms.input_dimensions);
    textureStore(output_texture, vec2<i32>(gid.xy), load_input(coord));
}
