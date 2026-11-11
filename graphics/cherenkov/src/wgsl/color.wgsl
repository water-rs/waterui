// GPU producers write premultiplied linear Display P3 into engine attachments.
// The conversions are filtrate-core's space fragment (`SPACE_*`,
// `space_*`), which precedes this file: the engine's own definition.

// Linear sRGB to linear Display P3.
fn cherenkov_linear_srgb(rgb: vec3<f32>) -> vec3<f32> {
    return SPACE_SRGB_TO_P3 * rgb;
}

// The sRGB transfer decode, mirrored through zero.
fn cherenkov_srgb_decode(rgb: vec3<f32>) -> vec3<f32> {
    return space_srgb_decode(rgb);
}

// Straight-alpha sRGB to premultiplied working color.
fn cherenkov_srgb(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(cherenkov_linear_srgb(cherenkov_srgb_decode(color.rgb)) * color.a, color.a);
}

// Encoded-domain premultiplied sRGB, as produced by browser compositors.
fn cherenkov_premultiplied_srgb(color: vec4<f32>) -> vec4<f32> {
    return space_from_srgb(color);
}
