// Photo effect: chrome — a saturation boost with a slight warm tint on
// straight-alpha colour, clamped to [0, f16 max].

struct WorkingSpace {
    luma: vec3<f32>,
}

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let luma = dot(straight, space.luma);
    let boosted = mix(vec3<f32>(luma), straight, 1.45);
    let warmed = boosted * vec3<f32>(1.05, 1.0, 0.95);
    return vec4<f32>(clamp(warmed, vec3<f32>(0.0), vec3<f32>(F16_MAX)) * color.a, color.a);
}
