
// The `from_srgb` stage: premultiplied sRGB back to premultiplied linear
// Display P3, after a stage that operates in sRGB. `space_from_srgb` comes
// from the space fragment that precedes this file.
fn apply(color: vec4<f32>) -> vec4<f32> {
    return space_from_srgb(color);
}
