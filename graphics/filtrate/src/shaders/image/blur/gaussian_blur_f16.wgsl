enable f16;

// One pass of a separable gaussian blur along `axis` ((1, 0) or (0, 1)).
// Tap weights integrate the gaussian over each pixel; the kernel radius is
// ceil(RADIUS_PER_SIGMA * sigma).

const RADIUS_PER_SIGMA: f32 = 4.0;

fn gaussian_erf(x: f32) -> f32 {
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

struct Params {
    sigma: f32,
    axis: vec2<f32>,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f16> {
    let pixel = floor(uv * size);
    let sigma = max(params.sigma, 0.001);
    let radius = max(i32(ceil(sigma * RADIUS_PER_SIGMA)), 0);
    if radius == 0 {
        return vec4<f16>(load(input, input_point_sampler, size, pixel));
    }

    let k = 1.0 / (sigma * 1.4142135624);
    var e_prev = gaussian_erf(0.5 * k);

    var sum = vec4<f32>(vec4<f16>(load(input, input_point_sampler, size, pixel)) * f16(e_prev));
    var weight_total = e_prev;
    for (var offset = 1; offset <= radius; offset++) {
        let e = gaussian_erf((f32(offset) + 0.5) * k);
        let weight = 0.5 * (e - e_prev);
        e_prev = e;
        let delta = params.axis * f32(offset);
        let half_weight = f16(weight);
        let left = vec4<f16>(load(input, input_point_sampler, size, pixel - delta));
        let right = vec4<f16>(load(input, input_point_sampler, size, pixel + delta));
        sum += vec4<f32>(left * half_weight) + vec4<f32>(right * half_weight);
        weight_total += 2.0 * weight;
    }
    return vec4<f16>(sum / weight_total);
}
