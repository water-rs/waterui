// Photo effect: noir — high-contrast luma desaturation of straight-alpha
// colour, stretched around the midtone and clamped to [0, f16 max].

struct WorkingSpace {
    luma: vec3<f32>,
}

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    let luma = dot(color.rgb / max(color.a, 1e-6), space.luma);
    let scaled = clamp((luma - 0.5) * 1.6 + 0.5, 0.0, F16_MAX);
    return vec4<f32>(vec3<f32>(scaled * color.a), color.a);
}
