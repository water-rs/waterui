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

// `color` — premultiplied, in the source colour `conversion` names — in
// the colour `conversion` targets.
static float4 convert(float4 color, constant CaptureCompositeConversion& conversion) {
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

fragment float4 capture_composite_fragment(
    CaptureCompositeVertexOut input [[stage_in]],
    texture2d<float> overlayTexture [[texture(0)]],
    sampler overlaySampler [[sampler(0)]],
    constant CaptureCompositeConversion& conversion [[buffer(0)]]) {
    return convert(overlayTexture.sample(overlaySampler, input.uv), conversion);
}

// # Ordered resolve
//
// The native raster cannot draw a GPU surface, so the capture measures how
// the native tree composites one instead: besides the base raster — every
// surface transparent — it rasterizes the tree with an opaque black and an
// opaque white probe layer standing exactly where the surface's
// presentation layer sits. Core Animation's compositing is affine in a
// layer's premultiplied colour — `over`, group opacity, masks, clipping
// coverage and antialiasing all scale or add it — so for a surface sample
// `s` (premultiplied, alpha `a`) every output channel `c` is
//
//     R0 + a·(RB − R0) + s_c·(RW − RB)
//
// with `s_alpha = a`: the probes measure, per pixel, how much of the
// surface the native content above it, its ancestors' group effects and
// its clip let through, and how much of what lies below it the surface
// covers. Surfaces whose destination rects overlap take distinct slots and
// one raster per combination of their probe states; the resolve contracts
// one slot at a time — the same weights, applied to three images that each
// still vary in the remaining slots.
//
// The weights sum to one in every channel, so where a probe does not
// reach — outside the surface's footprint — `RB = RW = R0` and the resolve
// is exactly the native raster.

// The per-draw resolve parameters, matching `ResolveParams` on the host —
// `repr(C)`, `float2` pairs followed by one `uint`.
struct CaptureResolveParams {
    // Where the render target's pixel (0, 0) sits in the capture target.
    float2 output_origin;
    // Where `x0`'s pixel (0, 0) sits in the capture target.
    float2 x0_origin;
    // Where `xb`'s and `xw`'s pixel (0, 0) sits in the capture target.
    float2 probe_origin;
    // The surface texture coordinate of capture-target point (0, 0)…
    float2 uv_offset;
    // …and its derivatives along the target's x and y axes: the inverse of
    // the surface's placement, rotation and scale included.
    float2 uv_dx;
    float2 uv_dy;
    // Whether a surface is bound — without one the draw copies `x0`.
    uint has_surface;
};

fragment float4 capture_resolve_fragment(
    CaptureCompositeVertexOut input [[stage_in]],
    texture2d<float> surfaceTexture [[texture(0)]],
    texture2d<float> x0 [[texture(1)]],
    texture2d<float> xb [[texture(2)]],
    texture2d<float> xw [[texture(3)]],
    sampler surfaceSampler [[sampler(0)]],
    constant CaptureCompositeConversion& imageConversion [[buffer(0)]],
    constant CaptureResolveParams& params [[buffer(1)]],
    constant CaptureCompositeConversion& surfaceConversion [[buffer(2)]]) {
    // The capture-target point this fragment's centre covers.
    float2 point = input.position.xy + params.output_origin;
    float4 base = convert(x0.read(uint2(point - params.x0_origin)), imageConversion);
    if (params.has_surface == 0) {
        return base;
    }
    uint2 probe = uint2(point - params.probe_origin);
    float4 black = convert(xb.read(probe), imageConversion);
    float4 white = convert(xw.read(probe), imageConversion);
    float2 uv = params.uv_offset + point.x * params.uv_dx + point.y * params.uv_dy;
    float4 surface = convert(surfaceTexture.sample(surfaceSampler, uv), surfaceConversion);
    // `surface.a` multiplies the alpha channel's `white - black`, which is
    // zero: both probes are opaque.
    return base + surface.a * (black - base) + surface * (white - black);
}
