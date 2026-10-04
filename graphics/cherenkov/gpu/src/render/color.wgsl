// GPU producers write premultiplied linear Display P3 into engine attachments.
fn cherenkov_linear_srgb(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.82246196 * rgb.r + 0.17753804 * rgb.g,
        0.0331942 * rgb.r + 0.9668058 * rgb.g,
        0.017082632 * rgb.r + 0.07239744 * rgb.g + 0.91051996 * rgb.b,
    );
}

fn cherenkov_srgb_decode(rgb: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(rgb);
    return sign(rgb) * select(pow((magnitude + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), magnitude / 12.92, magnitude <= vec3<f32>(0.04045));
}

// Straight-alpha sRGB to premultiplied working color.
fn cherenkov_srgb(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(cherenkov_linear_srgb(cherenkov_srgb_decode(color.rgb)) * color.a, color.a);
}

// Encoded-domain premultiplied sRGB, as produced by browser compositors.
fn cherenkov_premultiplied_srgb(color: vec4<f32>) -> vec4<f32> {
    if color.a == 0.0 {
        return vec4<f32>(0.0);
    }
    return cherenkov_srgb(vec4<f32>(color.rgb / color.a, color.a));
}
