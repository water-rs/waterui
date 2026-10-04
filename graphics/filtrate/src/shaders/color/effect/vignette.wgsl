// Vignette: darkens colour with distance from the centre. Distance is
// measured in isotropic space (1.0 = the shorter edge), so the vignette stays
// circular on non-square images. It depends on the pixel's position, so it
// is a spatial stage that reads only its own texel.

struct Params {
    radius: f32,
    softness: f32,
}

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let isotropic = size / min(size.x, size.y);
    let color = textureSampleLevel(input, input_point_sampler, uv, 0.0);
    let dist = distance(uv * isotropic, vec2<f32>(0.5) * isotropic);
    let softness = max(params.softness, 0.0001);
    let edge0 = max(params.radius - softness, 0.0);
    let edge1 = max(params.radius, edge0 + 0.0001);
    let vignette = 1.0 - smoothstep(edge0, edge1, dist);
    return vec4<f32>(color.rgb * vignette, color.a);
}
