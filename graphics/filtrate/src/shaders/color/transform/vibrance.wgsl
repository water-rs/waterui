// Vibrance: boosts muted straight-alpha colours more than vivid ones.
// Desaturation pivots on the working-space luma, like Saturation.

struct Params {
    amount: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}

fn apply(color: vec4<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let luma = dot(straight, space.luma);
    let max_c = max(max(straight.r, straight.g), straight.b);
    let saturation = max_c - luma;
    let response = 1.0 - saturation / max(max_c, 0.0001);
    let factor = max(0.0, 1.0 + params.amount * response);
    return vec4<f32>(mix(vec3<f32>(luma), straight, factor) * color.a, color.a);
}
