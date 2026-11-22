// Test stage: writes the shape's signed distance field to the red channel.

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, shape: texture_2d<f32>) -> vec4<f32> {
    let shape_size = vec2<f32>(textureDimensions(shape));
    let signed_distance = textureLoad(shape, vec2<i32>(floor(uv * shape_size)), 0).r;
    return vec4<f32>(signed_distance, 0.0, 0.0, 1.0);
}
