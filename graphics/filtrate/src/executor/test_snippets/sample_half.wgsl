// Test stage: one filtered sample at uv * 0.5 — a pure bilinear read.

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(input, input_sampler, uv * 0.5, 0.0);
}
