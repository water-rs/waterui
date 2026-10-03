#include <metal_stdlib>
using namespace metal;

// Composites the pieces of one WaterUI subtree capture into a single texture.
//
// # Orientation contract
//
// Every texture in the capture pipeline is *top-down*: texel row 0 is the
// visually topmost row. That is the convention `CAMetalLayer` presents, the
// convention wgpu renders in, and therefore the convention the filter and
// view-effect chains consume. the capture side establishes it on the
// `CARenderer` side by mirroring the captured layer tree vertically (see
// `captureLayerTransform`), because `CARenderer` otherwise writes its
// destination bottom-up.
//
// Both inputs are therefore already top-down when they reach this shader, so
// the pass is a plain identity copy. Metal puts NDC y = +1 at texel row 0 and
// NDC y = -1 at the last row, while texture coordinate v = 0 samples row 0, so
// identity means v must run *opposite* to NDC y.

struct CaptureCompositeVertexOut {
    float4 position [[position]];
    float2 uv;
};

vertex CaptureCompositeVertexOut capture_composite_vertex(uint vertexID [[vertex_id]]) {
    constexpr float2 positions[3] = {
        {-1.0, -1.0},
        {3.0, -1.0},
        {-1.0, 3.0},
    };
    CaptureCompositeVertexOut output;
    float2 position = positions[vertexID];
    output.position = float4(position, 0.0, 1.0);
    output.uv = float2(position.x + 1.0, 1.0 - position.y) * 0.5;
    return output;
}

fragment float4 capture_composite_fragment(
    CaptureCompositeVertexOut input [[stage_in]],
    texture2d<float> overlayTexture [[texture(0)]],
    sampler overlaySampler [[sampler(0)]]) {
    return overlayTexture.sample(overlaySampler, input.uv);
}
