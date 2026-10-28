#include <metal_stdlib>
using namespace metal;

// Composites the pieces of one WaterUI subtree capture into a single texture.
//
// # Orientation contract
//
// Every texture in the capture pipeline is *top-down*: texel row 0 is the
// visually topmost row. That is the convention `CAMetalLayer` presents, the
// convention wgpu renders in, and therefore the convention the filter and
// view-effect chains consume. The capture side establishes it on the
// raster side: the `CGContext` draw maps layer space onto the pixel
// destination through its CTM — scaling plus the platform's orientation
// normalization — so the buffer lands top-down.
//
// Both inputs are therefore already top-down when they reach this shader, so
// the pass samples a window of the producer texture — identity for the
// overlay, a crop for a surface that lands partially outside the capture.
// Metal puts NDC y = +1 at texel row 0 and NDC y = -1 at the last row, while
// texture coordinate v = 0 samples row 0, so the unit UV runs *opposite* to
// NDC y.

struct CaptureCompositeVertexOut {
    float4 position [[position]];
    float2 uv;
};

// The sampled window inside the producer texture, matching the `CompositeRegion`
// struct the host passes through vertex buffer 0 — `repr(C)`, so the field
// offsets line up with the `float2` pairs here.
struct CaptureCompositeRegion {
    float2 uv_origin;
    float2 uv_scale;
};

vertex CaptureCompositeVertexOut capture_composite_vertex(
    uint vertexID [[vertex_id]],
    constant CaptureCompositeRegion& region [[buffer(0)]]) {
    constexpr float2 positions[3] = {
        {-1.0, -1.0},
        {3.0, -1.0},
        {-1.0, 3.0},
    };
    CaptureCompositeVertexOut output;
    float2 position = positions[vertexID];
    output.position = float4(position, 0.0, 1.0);
    output.uv = region.uv_origin
        + float2(position.x + 1.0, 1.0 - position.y) * 0.5 * region.uv_scale;
    return output;
}

// The conversion from the sampled texture's colour to the target's, matching
// the `CompositeConversion` struct the host passes through fragment buffer 0
// — `repr(C, align(16))`, so the offsets and the 96-byte size line up.
//
// Each source and the target carry their own colour contract: the native
// raster holds (extended) sRGB-encoded values, an `_sRGB` texture samples as
// linear sRGB, a half-float surface holds linear extended Display P3. Every
// source reaches the pass's destination in the destination's own colour.
// The host blends the sources in extended sRGB-encoded values, as Core
// Animation composites a window's layers, and converts that blend into the
// target's colour in a pass of its own when the target stores another.
struct CaptureCompositeConversion {
    // Rows of the linear source-primaries → target-primaries matrix.
    float4 rows[3];
    // The sRGB transfer: slope, offset, scale (1 + offset), gamma.
    float4 transfer;
    // 1 / gamma, the linear-segment break, the encoded-segment break.
    float4 breaks;
    // Whether the conversion is the identity — the sample passes through.
    uint identity;
    // Whether the source carries the sRGB transfer.
    uint decode_source;
    // Whether the target expects the sRGB transfer.
    uint encode_target;
};

// The sRGB transfer of linear light, mirrored through zero so extended
// values stay extended.
static float3 srgb_encode(float3 linear, constant CaptureCompositeConversion& c) {
    float3 magnitude = abs(linear);
    float3 curve = select(
        magnitude * c.transfer.x,
        c.transfer.z * pow(magnitude, float3(c.breaks.x)) - c.transfer.y,
        magnitude > float3(c.breaks.y));
    return copysign(curve, linear);
}

// The inverse of `srgb_encode`.
static float3 srgb_decode(float3 encoded, constant CaptureCompositeConversion& c) {
    float3 magnitude = abs(encoded);
    float3 curve = select(
        magnitude / c.transfer.x,
        pow((magnitude + c.transfer.y) / c.transfer.z, float3(c.transfer.w)),
        magnitude > float3(c.breaks.z));
    return copysign(curve, encoded);
}

fragment float4 capture_composite_fragment(
    CaptureCompositeVertexOut input [[stage_in]],
    texture2d<float> overlayTexture [[texture(0)]],
    sampler overlaySampler [[sampler(0)]],
    constant CaptureCompositeConversion& conversion [[buffer(0)]]) {
    float4 color = overlayTexture.sample(overlaySampler, input.uv);
    if (conversion.identity != 0) {
        return color;
    }
    // Premultiplied in, premultiplied out: the transfer applies to the
    // straight colour. A pixel with zero alpha carries no colour.
    if (color.a == 0.0) {
        return float4(0.0);
    }
    float3 straight = color.rgb / color.a;
    if (conversion.decode_source != 0) {
        straight = srgb_decode(straight, conversion);
    }
    straight = float3(
        dot(conversion.rows[0].xyz, straight),
        dot(conversion.rows[1].xyz, straight),
        dot(conversion.rows[2].xyz, straight));
    if (conversion.encode_target != 0) {
        straight = srgb_encode(straight, conversion);
    }
    return float4(straight * color.a, color.a);
}
