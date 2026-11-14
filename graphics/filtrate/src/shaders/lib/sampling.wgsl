// Texel sampling helpers: an unfiltered read of the texel under a uv or at
// a pixel, with edge clamping. `size` is `input`'s extent in pixels — the
// `size` argument the spatial ABI provides — while an auxiliary `image`
// keeps its own `textureDimensions`.

// The texel of `input` holding `pixel` (a 0-based pixel coordinate),
// sampled at its centre through the point sampler.
fn load(input: texture_2d<f32>, input_point_sampler: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(input, input_point_sampler, (pixel + 0.5) / size, 0.0);
}

// The texel of `image` under `uv`, clamped to the image's extent.
fn texel_at(image: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(image));
    let coord = clamp(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(image, coord, 0);
}
