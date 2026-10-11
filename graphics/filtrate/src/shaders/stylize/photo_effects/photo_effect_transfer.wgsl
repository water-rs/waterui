// Photo effect: transfer — a warm fade with a soft sepia bias on
// straight-alpha colour, clamped to [0, f16 max].

struct WorkingSpace {
    luma: vec3<f32>,
}

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let luma = dot(straight, space.luma);
    let sepia = vec3<f32>(luma * 1.07, luma * 0.95, luma * 0.78);
    let blended = mix(straight, sepia, 0.55);
    return vec4<f32>(clamp(blended, vec3<f32>(0.0), vec3<f32>(F16_MAX)) * color.a, color.a);
}
