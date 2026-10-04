// Gamma: raises each straight-alpha channel to 1 / gamma. Negative values
// clamp to zero before the power.

struct Params {
    gamma: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let gamma = max(params.gamma, 0.001);
    // Fully transparent premultiplied texels have rgb == 0, so the guard
    // cannot manufacture colour there.
    let straight = max(color.rgb / max(color.a, 1e-6), vec3<f32>(0.0));
    return vec4<f32>(pow(straight, vec3<f32>(1.0 / gamma)) * color.a, color.a);
}
