// Sharpen: standalone laplacian-detail pass (spatial filter, cannot fuse).
//
// Untiled by design: the five overlapping laplacian taps hit the GPU
// texture cache, which benchmarks as fast as workgroup shared-memory
// tiling without the barriers and threadgroup-memory occupancy cost.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let amount = param(0u);
    let center_coord = map_to_input(gid.xy);

    let center = load_input(center_coord);
    let top = load_input(center_coord + vec2<i32>(0, -1));
    let bottom = load_input(center_coord + vec2<i32>(0, 1));
    let left = load_input(center_coord + vec2<i32>(-1, 0));
    let right = load_input(center_coord + vec2<i32>(1, 0));

    // Laplacian kernel; alpha keeps the centre's coverage.
    let laplacian = center * 4.0 - top - bottom - left - right;
    let result = center + laplacian * amount;
    textureStore(output_texture, vec2<i32>(gid.xy), vec4<f32>(result.rgb, center.a));
}
