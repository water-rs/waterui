#include <metal_stdlib>
using namespace metal;

// Composites one capture plan: every node is a textured quad in Core
// Animation paint order — a `renderInContext` raster segment, a GPU
// surface's producer texture, or a group's transient texture —
// transformed into the destination's root-layer space, clipped by
// ancestor rounded rects, faded by accumulated opacity, and optionally
// multiplied by a rasterized alpha mask. Blending is premultiplied
// source-over everywhere, so the fragment scales the whole color.
//
// # Orientation contract
//
// Every texture in the capture pipeline is *top-down*: texel row 0 is
// the visually topmost row — the convention `CAMetalLayer` presents and
// wgpu renders in. The raster side establishes it through the CTM's
// platform affine, which `dst_v_flip` mirrors for NDC and `source_v_flip`
// / `mask_v_flip` mirror for texture coordinates.

constant int MAX_CLIPS = 8;

struct NodeParams {
    // Node source space -> the destination's root-layer space.
    float4x4 transform;
    // Root space -> the mask owner's local space (`has_mask` only).
    float4x4 mask_inverse;
    // Root space -> each clip layer's local space.
    float4x4 clip_inverses[MAX_CLIPS];
    // Each clip's bounds in its layer's local space.
    float4   clip_rects[MAX_CLIPS];
    // Per-corner radii for each clip, matching `maskedCorners`.
    float4   clip_radii[MAX_CLIPS];
    // The quad's rect in node source space (x, y, w, h).
    float4   source_rect;
    // The mask texture's coverage in owner-local space.
    float4   mask_extent;
    // The pass extent's origin in root space.
    float2   dst_origin;
    // Points -> pixels.
    float2   dst_scale;
    // The destination's pixel size.
    float2   dst_pixels;
    // Accumulated draw opacity.
    float    opacity;
    // Nonzero samples the source texture V-flipped.
    float    source_v_flip;
    // Nonzero samples the mask texture V-flipped.
    float    mask_v_flip;
    // The destination's Y convention: +1 puts the root space's max-Y
    // edge on texel row 0 (macOS), -1 the min-Y edge (iOS).
    float    dst_v_flip;
    // How many clip shapes apply.
    uint     clip_count;
    // Nonzero binds and applies the mask texture.
    uint     has_mask;
};

struct VertexOut {
    float4 position [[position]];
    float2 uv;
    float2 root_xy;
};

vertex VertexOut capture_composite_vertex(
    uint vertexID [[vertex_id]],
    constant NodeParams& params [[buffer(0)]]) {
    const float2 ts[6] = {
        {0.0, 0.0}, {1.0, 0.0}, {0.0, 1.0},
        {1.0, 0.0}, {1.0, 1.0}, {0.0, 1.0},
    };
    float2 t = ts[vertexID];
    float2 local = params.source_rect.xy + t * params.source_rect.zw;
    float4 root4 = params.transform * float4(local, 0.0, 1.0);
    float2 root = root4.xy / root4.w;
    float2 rel = (root - params.dst_origin) * params.dst_scale;
    VertexOut output;
    output.position = float4(
        2.0 * rel.x / params.dst_pixels.x - 1.0,
        params.dst_v_flip * (2.0 * rel.y / params.dst_pixels.y - 1.0),
        0.0,
        1.0);
    output.uv = float2(t.x, mix(t.y, 1.0 - t.y, params.source_v_flip));
    output.root_xy = root;
    return output;
}

fragment float4 capture_composite_fragment(
    VertexOut input [[stage_in]],
    texture2d<float> sourceTexture [[texture(0)]],
    texture2d<float> maskTexture [[texture(1)]],
    sampler sourceSampler [[sampler(0)]],
    constant NodeParams& params [[buffer(0)]]) {
    float alpha = params.opacity;
    for (uint i = 0; i < params.clip_count; i++) {
        float4 lp4 = params.clip_inverses[i] * float4(input.root_xy, 0.0, 1.0);
        float2 lp = lp4.xy / lp4.w;
        float4 rect = params.clip_rects[i];
        float2 center = rect.xy + 0.5 * rect.zw;
        float2 half_ = 0.5 * rect.zw;
        // The corner the fragment is nearest decides the radius, in the
        // clip layer's own coordinate naming.
        float radius;
        if (lp.x < center.x) {
            radius = lp.y < center.y ? params.clip_radii[i].x : params.clip_radii[i].z;
        } else {
            radius = lp.y < center.y ? params.clip_radii[i].y : params.clip_radii[i].w;
        }
        radius = clamp(radius, 0.0, min(half_.x, half_.y));
        float2 inside = max(abs(lp - center) - (half_ - radius), float2(0.0));
        float distance = length(inside) - radius;
        float width = fwidth(distance) + 1e-4;
        alpha *= 1.0 - smoothstep(-width, width, distance);
    }
    if (params.has_mask != 0u) {
        float4 mp4 = params.mask_inverse * float4(input.root_xy, 0.0, 1.0);
        float2 mp = mp4.xy / mp4.w;
        float2 mt = (mp - params.mask_extent.xy) / params.mask_extent.zw;
        float2 mask_uv = float2(mt.x, mix(mt.y, 1.0 - mt.y, params.mask_v_flip));
        alpha *= maskTexture.sample(sourceSampler, mask_uv).a;
    }
    return sourceTexture.sample(sourceSampler, input.uv) * alpha;
}
