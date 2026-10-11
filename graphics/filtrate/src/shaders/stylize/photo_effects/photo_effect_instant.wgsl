// Photo effect: instant — instant-camera warmth with reduced contrast, on
// straight-alpha colour, clamped to [0, f16 max].

struct WorkingSpace {
    luma: vec3<f32>,
}

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let warmed = straight * vec3<f32>(1.10, 1.02, 0.85) + vec3<f32>(0.05, 0.04, 0.0);
    let luma = dot(warmed, space.luma);
    // Pull contrast down toward the mid-tone luma.
    let softened = mix(vec3<f32>(luma), warmed, 0.7);
    return vec4<f32>(clamp(softened, vec3<f32>(0.0), vec3<f32>(F16_MAX)) * color.a, color.a);
}
