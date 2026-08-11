// Vortex distortion: like twirl, but with a quadratic falloff that
// concentrates the rotation near the center.
//
// Parameters: center.x, center.y (uv space), radius (isotropic units,
// 1.0 = shorter output edge), angle (degrees). Distances are measured in
// isotropic space so the vortex stays circular on non-square targets. The
// rotation is applied as a 2x2 matrix — no atan2 polar round-trip.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = output_uv(gid.xy);
    let center = vec2<f32>(param(0u), param(1u));
    let radius = max(param(2u), 0.001);
    let angle = param(3u) * DEGREES_TO_RADIANS;

    let center_iso = to_isotropic(center);
    let delta = to_isotropic(uv) - center_iso;
    let dist = length(delta);
    var sample_uv = uv;
    if dist < radius {
        let t = (radius - dist) / radius;
        sample_uv = from_isotropic(center_iso + rotate2(delta, angle * t * t));
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(sample_uv));
}
