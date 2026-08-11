// Unsharp mask, second (vertical) pass.
//
// The first pass is the shared horizontal box blur; this pass finishes the
// separable blur vertically, then sharpens against the untouched original:
// result = original + (original - blurred) * amount. The separable pair
// costs 2(2r+1) taps per pixel instead of (2r+1)^2.
//
// Parameters: radius (param 0, shared with the horizontal pass), amount
// (param 1).

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);
    let radius = max(i32(round(param(0u))), 0);
    let amount = max(param(1u), 0.0);

    var sum = load_input(center);
    for (var offset = 1; offset <= radius; offset++) {
        sum += load_input(center + vec2<i32>(0, -offset))
            + load_input(center + vec2<i32>(0, offset));
    }
    let blurred = sum / f32(2 * radius + 1);

    let original = load_original(map_to_original(gid.xy));
    let sharpened = original.rgb + (original.rgb - blurred.rgb) * amount;
    textureStore(output_texture, vec2<i32>(gid.xy), vec4<f32>(sharpened, original.a));
}
