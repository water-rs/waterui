// Temperature and tint: shifts straight-alpha colour along the blue-yellow
// and green-magenta axes, clamping negative results to zero.

struct Params {
    temperature: f32,
    tint: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let warmed = straight * vec3<f32>(
        1.0 + params.temperature * 0.1,
        1.0,
        1.0 - params.temperature * 0.1,
    );
    let tinted = warmed + params.tint * vec3<f32>(0.05, -0.05, 0.05);
    return vec4<f32>(max(tinted, vec3<f32>(0.0)) * color.a, color.a);
}
