
// The `to_srgb` stage: premultiplied linear Display P3 to premultiplied
// sRGB (sRGB primaries, sRGB transfer), before a stage that operates in
// sRGB. `space_to_srgb` comes from the space fragment that precedes this
// file.
fn apply(color: vec4<f32>) -> vec4<f32> {
    return space_to_srgb(color);
}
