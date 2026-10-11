// Bump distortion: magnifies the image inside a circular region.
//
// Parameters: centre (uv), radius (isotropic units, 1.0 = the shorter edge),
// scale. Distances are measured in isotropic space so the bump stays
// circular on non-square images.

struct Params {
    center_x: f32,
    center_y: f32,
    radius: f32,
    scale: f32,
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let isotropic = size / min(size.x, size.y);
    let radius = max(params.radius, 0.001);

    let center = vec2<f32>(params.center_x, params.center_y) * isotropic;
    let delta = uv * isotropic - center;
    let dist = length(delta);
    var sample_uv = uv;
    if dist < radius {
        let t = 1.0 - dist / radius;
        let factor = 1.0 + params.scale * t * t;
        sample_uv = (center + delta / factor) / isotropic;
    }
    return textureSampleLevel(input, input_sampler, sample_uv, 0.0);
}
