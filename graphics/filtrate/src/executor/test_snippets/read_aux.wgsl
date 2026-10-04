// Test stage: passes the aux0 texel at uv through unchanged — the output is
// the aux image, so its precision shows in the readback.

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, aux0: texture_2d<f32>) -> vec4<f32> {
    let aux_size = vec2<f32>(textureDimensions(aux0));
    let coord = vec2<i32>(floor(uv * aux_size));
    return textureLoad(aux0, coord, 0);
}
