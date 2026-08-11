// Separable horizontal box-blur pass.
//
// Untiled by design: overlapping taps hit the GPU texture cache, which
// benchmarks faster than workgroup shared-memory tiling on tiler GPUs.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);
    let radius = max(i32(round(param(0u))), 0);
    if radius == 0 {
        textureStore(output_texture, vec2<i32>(gid.xy), load_input(center));
        return;
    }

    var sum = load_input(center);
    for (var offset = 1; offset <= radius; offset++) {
        sum += load_input(center + vec2<i32>(-offset, 0))
            + load_input(center + vec2<i32>(offset, 0));
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sum / f32(2 * radius + 1));
}
