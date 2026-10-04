// Converts premultiplied linear Display P3 to premultiplied sRGB (sRGB
// primaries, sRGB transfer), around stages that operate in sRGB. The
// transfer is mirrored through zero, so extended values stay extended.

// Linear Display P3 to linear sRGB (both D65), columns first.
const P3_TO_SRGB: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.2249402, -0.0420570, -0.0196376),
    vec3<f32>(-0.2249402, 1.0420570, -0.0786360),
    vec3<f32>(0.0, 0.0, 1.0982736),
);

fn encode(value: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(value);
    let curve = select(
        magnitude * 12.92,
        1.055 * pow(magnitude, vec3<f32>(1.0 / 2.4)) - 0.055,
        magnitude > vec3<f32>(0.0031308),
    );
    return sign(value) * curve;
}

fn apply(color: vec4<f32>) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    return vec4<f32>(encode(P3_TO_SRGB * straight) * color.a, color.a);
}
