// Converts premultiplied sRGB (sRGB primaries, sRGB transfer) back to
// premultiplied linear Display P3, after stages that operate in sRGB. The
// transfer is mirrored through zero, so extended values stay extended.

// Linear sRGB to linear Display P3 (both D65), columns first.
const SRGB_TO_P3: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.8224620, 0.0331942, 0.0170826),
    vec3<f32>(0.1775380, 0.9668058, 0.0723974),
    vec3<f32>(0.0, 0.0, 0.9105199),
);

fn decode(encoded: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(encoded);
    let curve = select(
        magnitude / 12.92,
        pow((magnitude + 0.055) / 1.055, vec3<f32>(2.4)),
        magnitude > vec3<f32>(0.04045),
    );
    return sign(encoded) * curve;
}

fn apply(color: vec4<f32>) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    return vec4<f32>(SRGB_TO_P3 * decode(straight) * color.a, color.a);
}
