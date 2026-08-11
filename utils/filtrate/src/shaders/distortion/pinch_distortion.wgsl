// Pinch distortion: pulls the image toward a center point.
//
// Parameters: center.x, center.y (uv space), radius (isotropic units,
// 1.0 = shorter output edge), scale. Distances are measured in isotropic
// space so the pinch stays circular on non-square targets.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = output_uv(gid.xy);
    let center = vec2<f32>(param(0u), param(1u));
    let radius = max(param(2u), 0.001);
    let scale = param(3u);

    let center_iso = to_isotropic(center);
    let delta = to_isotropic(uv) - center_iso;
    let dist = length(delta);
    var sample_uv = uv;
    // The center texel maps to itself; guarding also keeps normalize() away
    // from the zero vector.
    if dist < radius && dist > 1e-6 {
        let t = dist / radius;
        let factor = pow(t, 1.0 + scale);
        sample_uv = from_isotropic(center_iso + normalize(delta) * factor * radius);
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(sample_uv));
}
