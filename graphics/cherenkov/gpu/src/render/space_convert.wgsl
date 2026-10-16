// A filter over an sRGB-encoded scope: the scope stores premultiplied
// encoded sRGB, while every filter works in the premultiplied linear
// working space. `fs_decode` moves the scope's texels into the filter's
// input and `fs_encode` moves its output back, texel for texel, through
// the space fragment (`space_from_srgb` / `space_to_srgb`) that precedes
// this file.

@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_decode(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return space_from_srgb(textureLoad(source, vec2<i32>(floor(position.xy)), 0));
}

@fragment
fn fs_encode(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return space_to_srgb(textureLoad(source, vec2<i32>(floor(position.xy)), 0));
}
