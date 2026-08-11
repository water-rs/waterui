// 3x3 morphological dilation: per-channel maximum over the neighbourhood.
//
// The accumulator is seeded from the centre texel, so the operator is
// range-agnostic — HDR values above 1.0 and negative scene-referred values
// survive. All four channels dilate together (premultiplied-alpha
// consistent).

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);

    var acc = load_input(center);
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            acc = max(acc, load_input(center + vec2<i32>(dx, dy)));
        }
    }
    textureStore(output_texture, vec2<i32>(gid.xy), acc);
}
