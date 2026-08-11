// Motion blur: directional blur along an angle.
//
// Parameters: radius (pixels), angle (degrees). Taps interpolate
// bilinearly so off-axis directions produce a smooth streak instead of
// duplicated nearest texels.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let mapped = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;

    let radius = max(i32(round(param(0u))), 0);
    if radius == 0 {
        textureStore(output_texture, vec2<i32>(gid.xy), load_input(map_to_input(gid.xy)));
        return;
    }

    let angle = param(1u) * DEGREES_TO_RADIANS;
    let direction = vec2<f32>(cos(angle), sin(angle));

    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let sample_uv = (mapped + direction * f32(i)) / uniforms.input_dimensions;
        let weight = 1.0 - abs(f32(i)) / f32(radius + 1);
        sum += sample_input_bilinear(sample_uv) * weight;
        total_weight += weight;
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sum / total_weight);
}
