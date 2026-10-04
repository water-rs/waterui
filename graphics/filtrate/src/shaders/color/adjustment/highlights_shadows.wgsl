// Highlights and shadows: lifts shadows and compresses highlights, weighted
// by the straight-alpha luma.

struct Params {
    highlights: f32,
    shadows: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}

fn apply(color: vec4<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let luma = dot(straight, space.luma);
    let shadow_mask = pow(clamp(1.0 - luma, 0.0, 1.0), 2.0);
    let highlight_mask = pow(clamp(luma, 0.0, 1.0), 2.0);
    let shadowed = straight * (1.0 + params.shadows * shadow_mask);
    let adjusted = shadowed * (1.0 - params.highlights * highlight_mask);
    return vec4<f32>(max(adjusted, vec3<f32>(0.0)) * color.a, color.a);
}
