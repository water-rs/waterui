// Test stage: scales the input by the clip shape's coverage mask.

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, shape: texture_2d<f32>) -> vec4<f32> {
    let shape_size = vec2<f32>(textureDimensions(shape));
    let coverage = textureLoad(shape, vec2<i32>(floor(uv * shape_size)), 0).r;
    return textureSampleLevel(input, input_point_sampler, uv, 0.0) * coverage;
}
