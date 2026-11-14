// Photo effect: fade — lifted blacks and mild desaturation of straight-alpha
// colour, clamped to [0, f16 max].

struct WorkingSpace {
    luma: vec3<f32>,
}

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    let lifted = mix(vec3<f32>(0.10), color.rgb / max(color.a, 1e-6), 0.85);
    let luma = dot(lifted, space.luma);
    let desaturated = mix(vec3<f32>(luma), lifted, 0.75);
    return vec4<f32>(clamp(desaturated, vec3<f32>(0.0), vec3<f32>(F16_MAX)) * color.a, color.a);
}
