// Tone curve: reshapes each channel of the colour as sampled — a gamma
// curve plus shadow, midtone and highlight lifts — clamped to [0, 1] and
// mixed in by `amount`.

struct Params {
    shadows: f32,
    midtones: f32,
    highlights: f32,
    gamma: f32,
    amount: f32,
}

fn curve(x: f32, params: Params) -> f32 {
    let g = pow(clamp(x, 0.0, 1.0), 1.0 / max(params.gamma, 0.001));
    let shadow_weight = (1.0 - g) * (1.0 - g);
    let highlight_weight = g * g;
    let mid_weight = 1.0 - abs(g * 2.0 - 1.0);
    let curved = g
        + params.shadows * shadow_weight
        + params.midtones * mid_weight
        + params.highlights * highlight_weight;
    return clamp(curved, 0.0, 1.0);
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let curved = vec3<f32>(curve(color.r, params), curve(color.g, params), curve(color.b, params));
    return vec4<f32>(mix(color.rgb, curved, clamp(params.amount, 0.0, 1.0)), color.a);
}
