// Photo effect: process — a cool cast with crushed highlights on
// straight-alpha colour, clamped to [0, f16 max].

const F16_MAX: f32 = 65504.0;

fn apply(color: vec4<f32>) -> vec4<f32> {
    let cooled = color.rgb / max(color.a, 1e-6) * vec3<f32>(0.90, 0.95, 1.10);
    let crushed = min(cooled, vec3<f32>(0.92, 0.94, 0.96));
    return vec4<f32>(clamp(crushed, vec3<f32>(0.0), vec3<f32>(F16_MAX)) * color.a, color.a);
}
