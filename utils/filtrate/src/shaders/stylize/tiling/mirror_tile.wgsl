// Mirror tile: repeats the image with alternate tiles mirrored, so tile
// seams are continuous. Even-indexed tiles (including the first) pass
// through unmirrored — the identity configuration (repeat = 1) reproduces
// the input.
//
// Parameters: repeat_x, repeat_y.

fn mirror_repeat(v: f32) -> f32 {
    let tiled = fract(v);
    let odd_tile = (i32(floor(v)) & 1) != 0;
    return select(tiled, 1.0 - tiled, odd_tile);
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let repeat_x = max(param(0u), 1.0);
    let repeat_y = max(param(1u), 1.0);
    let uv = output_uv(gid.xy);
    let tiled_uv = vec2<f32>(
        mirror_repeat(uv.x * repeat_x),
        mirror_repeat(uv.y * repeat_y),
    );
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(tiled_uv));
}
