// 3x3 morphological gradient: per-channel (max - min) over the
// neighbourhood. Highlights region boundaries.
//
// Both accumulators are seeded from the centre texel, so the operator is
// range-agnostic — HDR values above 1.0 and negative scene-referred values
// produce correct gradients. Alpha keeps the centre's coverage.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);

    let centre_texel = load_input(center);
    var lo = centre_texel.rgb;
    var hi = centre_texel.rgb;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            let texel = load_input(center + vec2<i32>(dx, dy)).rgb;
            lo = min(lo, texel);
            hi = max(hi, texel);
        }
    }
    textureStore(output_texture, vec2<i32>(gid.xy), vec4<f32>(hi - lo, centre_texel.a));
}
