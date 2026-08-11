// Zoom blur: radial streak toward a focal point.
//
// Parameters: amount, center.x, center.y (uv space). Taps interpolate
// bilinearly so small amounts produce a smooth streak instead of a handful
// of duplicated nearest texels.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = output_uv(gid.xy);
    let amount = max(param(0u), 0.0);
    let center = vec2<f32>(param(1u), param(2u));
    if amount <= 0.0001 {
        textureStore(output_texture, vec2<i32>(gid.xy), load_input(map_to_input(gid.xy)));
        return;
    }

    let direction = center - uv;
    let samples: i32 = 12;
    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;
    for (var i = 0; i < samples; i++) {
        let t = f32(i) / f32(samples - 1);
        let sample_uv = uv + direction * amount * t;
        let weight = 1.0 - t * 0.65;
        sum += sample_input_bilinear(sample_uv) * weight;
        total_weight += weight;
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sum / total_weight);
}
