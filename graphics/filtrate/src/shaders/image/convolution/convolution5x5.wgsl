// 5x5 convolution with a caller-supplied kernel, row-major from the
// top-left. The kernel is not normalised. All four channels are convolved
// together, which is the correct linear operation on premultiplied colour; a
// kernel summing to zero therefore also zeroes coverage.

struct Params {
    k0: f32,
    k1: f32,
    k2: f32,
    k3: f32,
    k4: f32,
    k5: f32,
    k6: f32,
    k7: f32,
    k8: f32,
    k9: f32,
    k10: f32,
    k11: f32,
    k12: f32,
    k13: f32,
    k14: f32,
    k15: f32,
    k16: f32,
    k17: f32,
    k18: f32,
    k19: f32,
    k20: f32,
    k21: f32,
    k22: f32,
    k23: f32,
    k24: f32,
}
fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let pixel = floor(uv * size);
    var kernel = array<f32, 25>(
        params.k0, params.k1, params.k2, params.k3, params.k4,
        params.k5, params.k6, params.k7, params.k8, params.k9,
        params.k10, params.k11, params.k12, params.k13, params.k14,
        params.k15, params.k16, params.k17, params.k18, params.k19,
        params.k20, params.k21, params.k22, params.k23, params.k24,
    );

    var acc = vec4<f32>(0.0);
    var idx: u32 = 0u;
    for (var dy: i32 = -2; dy <= 2; dy = dy + 1) {
        for (var dx: i32 = -2; dx <= 2; dx = dx + 1) {
            let offset = vec2<f32>(f32(dx), f32(dy));
            acc += load(input, input_point_sampler, size, pixel + offset) * kernel[idx];
            idx = idx + 1u;
        }
    }
    return acc;
}
