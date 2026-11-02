// The working-space (linear Display P3) <-> sRGB conversion, rendered by
// filtrate-core's build script from `space/constants.rs`, the one
// definition of these numbers on the CPU and the GPU.

// Linear Display P3 to linear sRGB (both D65), columns first.
const SPACE_P3_TO_SRGB: mat3x3<f32> = mat3x3<f32>(
{%- for column in p3_to_srgb %}
    vec3<f32>({{ column[0] }}, {{ column[1] }}, {{ column[2] }}),
{%- endfor %}
);

// Linear sRGB to linear Display P3 (both D65), columns first.
const SPACE_SRGB_TO_P3: mat3x3<f32> = mat3x3<f32>(
{%- for column in srgb_to_p3 %}
    vec3<f32>({{ column[0] }}, {{ column[1] }}, {{ column[2] }}),
{%- endfor %}
);

// The sRGB transfer (IEC 61966-2-1).
const SPACE_SRGB_SLOPE: f32 = {{ slope }};
const SPACE_SRGB_OFFSET: f32 = {{ offset }};
const SPACE_SRGB_SCALE: f32 = {{ scale }};
const SPACE_SRGB_GAMMA: f32 = {{ gamma }};
const SPACE_SRGB_INVERSE_GAMMA: f32 = {{ inverse_gamma }};
const SPACE_SRGB_LINEAR_BREAK: f32 = {{ linear_break }};
const SPACE_SRGB_ENCODED_BREAK: f32 = {{ encoded_break }};

// The sRGB transfer of linear channels, mirrored through zero so extended
// values stay extended.
fn space_srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(linear);
    let curve = select(
        magnitude * SPACE_SRGB_SLOPE,
        SPACE_SRGB_SCALE * pow(magnitude, vec3<f32>(SPACE_SRGB_INVERSE_GAMMA)) - SPACE_SRGB_OFFSET,
        magnitude > vec3<f32>(SPACE_SRGB_LINEAR_BREAK),
    );
    return sign(linear) * curve;
}

// The inverse of `space_srgb_encode`.
fn space_srgb_decode(encoded: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(encoded);
    let curve = select(
        magnitude / SPACE_SRGB_SLOPE,
        pow((magnitude + SPACE_SRGB_OFFSET) / SPACE_SRGB_SCALE, vec3<f32>(SPACE_SRGB_GAMMA)),
        magnitude > vec3<f32>(SPACE_SRGB_ENCODED_BREAK),
    );
    return sign(encoded) * curve;
}

// Premultiplied working-space colour to premultiplied sRGB. A pixel with
// zero alpha carries no colour and becomes transparent black.
fn space_to_srgb(color: vec4<f32>) -> vec4<f32> {
    if color.a == 0.0 {
        return vec4<f32>(0.0);
    }
    let straight = color.rgb / color.a;
    return vec4<f32>(space_srgb_encode(SPACE_P3_TO_SRGB * straight) * color.a, color.a);
}

// Premultiplied sRGB to premultiplied working-space colour, the inverse of
// `space_to_srgb`.
fn space_from_srgb(color: vec4<f32>) -> vec4<f32> {
    if color.a == 0.0 {
        return vec4<f32>(0.0);
    }
    let straight = color.rgb / color.a;
    return vec4<f32>(SPACE_SRGB_TO_P3 * space_srgb_decode(straight) * color.a, color.a);
}
