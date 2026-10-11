// Vortex distortion: like twirl, but with a quadratic falloff that
// concentrates the rotation near the centre.
//
// Parameters: centre (uv), radius (isotropic units, 1.0 = the shorter edge),
// angle (degrees).

struct Params {
    center_x: f32,
    center_y: f32,
    radius: f32,
    angle: f32,
}

const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;
fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let isotropic = size / min(size.x, size.y);
    let radius = max(params.radius, 0.001);
    let angle = params.angle * DEGREES_TO_RADIANS;

    let center = vec2<f32>(params.center_x, params.center_y) * isotropic;
    let delta = uv * isotropic - center;
    let dist = length(delta);
    var sample_uv = uv;
    if dist < radius {
        let t = (radius - dist) / radius;
        sample_uv = (center + rotate2(delta, angle * t * t)) / isotropic;
    }
    return textureSampleLevel(input, input_sampler, sample_uv, 0.0);
}
