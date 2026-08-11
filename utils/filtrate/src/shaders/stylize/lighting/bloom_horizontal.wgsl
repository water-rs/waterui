// Bloom, first (horizontal) pass: extracts thresholded highlight energy and
// box-accumulates it horizontally.
//
// Each tap contributes `color * max(luma - threshold, 0)` and the pass
// normalizes by tap count, so the threshold controls the MAGNITUDE of the
// glow — a fully sub-threshold neighbourhood contributes exactly zero.
// (Normalizing by summed weights instead would cancel the threshold and
// bloom unconditionally.)
//
// Parameters: radius (param 0), threshold (param 1).

fn bright_energy(sample_color: vec4<f32>, threshold: f32) -> vec3<f32> {
    let bright = max(luminance(sample_color.rgb) - threshold, 0.0);
    return sample_color.rgb * bright;
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);
    let radius = max(i32(round(param(0u))), 1);
    let threshold = max(param(1u), 0.0);

    var sum = vec3<f32>(0.0);
    for (var x = -radius; x <= radius; x++) {
        sum += bright_energy(load_input(center + vec2<i32>(x, 0)), threshold);
    }
    let energy = sum / f32(2 * radius + 1);
    textureStore(output_texture, vec2<i32>(gid.xy), vec4<f32>(energy, 0.0));
}
