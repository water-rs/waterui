// Luma gradient magnitude from a 3x3 operator at a configurable sampling
// radius, scaled by `amount`: Sobel (centre weight 2), Prewitt (centre
// weight 1) and edge work share it. The output is a grey scaled by the centre
// pixel's alpha, so premultiplied coverage stays valid.

struct Params {
    radius: f32,
    amount: f32,
    centre_weight: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}
fn luma_at(input: texture_2d<f32>, input_point_sampler: sampler, size: vec2<f32>, pixel: vec2<f32>, luma: vec3<f32>) -> f32 {
    return dot(load(input, input_point_sampler, size, pixel).rgb, luma);
}

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let pixel = floor(uv * size);
    let r = f32(max(i32(round(params.radius)), 1));
    let amount = max(params.amount, 0.0);
    let w = params.centre_weight;

    let tl = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(-r, -r), space.luma);
    let tc = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(0.0, -r), space.luma);
    let tr = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(r, -r), space.luma);
    let ml = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(-r, 0.0), space.luma);
    let mr = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(r, 0.0), space.luma);
    let bl = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(-r, r), space.luma);
    let bc = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(0.0, r), space.luma);
    let br = luma_at(input, input_point_sampler, size, pixel + vec2<f32>(r, r), space.luma);

    let gx = -tl - w * ml - bl + tr + w * mr + br;
    let gy = -tl - w * tc - tr + bl + w * bc + br;
    let edge = clamp(length(vec2<f32>(gx, gy)) * amount, 0.0, 1.0);

    let centre_alpha = load(input, input_point_sampler, size, pixel).a;
    return vec4<f32>(vec3<f32>(edge * centre_alpha), centre_alpha);
}
