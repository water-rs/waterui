// Gloom, second (vertical) pass: finishes the separable box blur of the
// highlight energy and subtracts it from the original, dulling highlights.
//
// The original texture is the input of this filter's FIRST stage (bound by
// the runtime), read through map_to_original so mismatched sizes stay
// correct.
//
// Parameters: radius (param 0), intensity (param 1).

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let radius = max(i32(round(param(0u))), 1);
    let intensity = max(param(1u), 0.0);

    var sum = vec3<f32>(0.0);
    for (var y = -radius; y <= radius; y++) {
        sum += load_input(coord + vec2<i32>(0, y)).rgb;
    }
    let gloom = sum / f32(2 * radius + 1);

    let base = load_original(map_to_original(gid.xy));
    textureStore(
        output_texture,
        coord,
        vec4<f32>(max(base.rgb - gloom * intensity, vec3<f32>(0.0)), base.a),
    );
}
