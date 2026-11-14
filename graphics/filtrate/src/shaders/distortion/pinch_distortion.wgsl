// Pinch distortion: pulls the image toward a centre point.
//
// Parameters: centre (uv), radius (isotropic units, 1.0 = the shorter edge),
// scale. Distances are measured in isotropic space so the pinch stays
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
    // The centre texel maps to itself; the guard also keeps normalize() away
    // from the zero vector.
    if dist < radius && dist > 1e-6 {
        let t = dist / radius;
        let factor = pow(t, 1.0 + params.scale);
        sample_uv = (center + normalize(delta) * factor * radius) / isotropic;
    }
    return textureSampleLevel(input, input_sampler, sample_uv, 0.0);
}
