#include <metal_stdlib>
using namespace metal;

// Paints a mesh gradient into a texture: the Apple realization of the
// recorded mesh paint (graphics/cherenkov/docs/mesh-paint.md).
//
// A mesh is row-major bilinear quadrilateral patches. For each pixel the
// pass inverts the patches, last row-major patch first, at the pixel's
// point in the mesh's own space; the first patch that contains the point
// owns the pixel, and a folded patch takes the inverse with the greater
// vertical coordinate, then the greater horizontal one. Colours are
// premultiplied linear Display P3, interpolated bilinearly (or smoothstep
// weighted). A pixel outside every patch is transparent, and the pass
// stores without blending. A pixel the patches' boundary crosses averages
// eight sub-pixel samples, so that boundary is antialiased; the enclosing
// shape's mask supplies the shape's own coverage.
//
// The solve matches the WGSL `mesh_uv` of the Cherenkov GPU engine,
// which in turn matches the f64 oracle.
//
// The texture is top-down: row 0 is the visually topmost row.

struct MeshPatch {
    // Corners 00 and 10, then 01 and 11, as x, y pairs.
    float4 top;
    float4 bottom;
    // Premultiplied colours of corners 00, 10, 01, 11.
    float4 colors[4];
};

// Matches `MeshUniforms` in mesh.rs, `repr(C, align(16))`.
struct MeshUniforms {
    // The rectangle the texture covers, in the layer's space: x, y, w, h.
    float4 rect;
    // The inverse of the mesh's placement: a, b, c, d of
    // `mesh = linear * layer + offset`, column-major as kurbo's coefficients.
    float4 linear_part;
    float2 offset;
    uint count;
    uint smooth_color;
};

struct MeshVertexOut {
    float4 position [[position]];
    float2 point;
};

vertex MeshVertexOut mesh_gradient_vertex(
    uint vertexID [[vertex_id]],
    constant MeshUniforms& uniforms [[buffer(0)]]) {
    constexpr float2 positions[3] = {
        {-1.0, -1.0},
        {3.0, -1.0},
        {-1.0, 3.0},
    };
    float2 position = positions[vertexID];
    // NDC y = +1 is texel row 0; texture v = 0 is the top.
    float2 unit = float2(position.x + 1.0, 1.0 - position.y) * 0.5;
    MeshVertexOut output;
    output.position = float4(position, 0.0, 1.0);
    float2 layer_point = uniforms.rect.xy + unit * uniforms.rect.zw;
    float4 m = uniforms.linear_part;
    output.point = float2(
        m.x * layer_point.x + m.z * layer_point.y + uniforms.offset.x,
        m.y * layer_point.x + m.w * layer_point.y + uniforms.offset.y);
    return output;
}

static float mesh_cross(float2 a, float2 b) {
    return a.x * b.y - a.y * b.x;
}

// (u, v, found) for `point` in the patch whose top and bottom edges are
// `top` and `bottom`; found is 0 when no inverse lies in the unit square.
static float3 mesh_uv(float2 point, float4 top, float4 bottom) {
    float2 horizontal = top.zw - top.xy;
    float2 vertical = bottom.xy - top.xy;
    float2 bend = bottom.zw - bottom.xy - horizontal;
    float2 delta = point - top.xy;
    float qa = -mesh_cross(vertical, bend);
    float qb = mesh_cross(delta, bend) - mesh_cross(vertical, horizontal);
    float qc = mesh_cross(delta, horizontal);
    float2 roots = float2(-1.0);
    if (qa == 0.0) {
        if (qb == 0.0) {
            return float3(0.0);
        }
        roots = float2(-qc / qb);
    } else {
        float discriminant = qb * qb - 4.0 * qa * qc;
        if (discriminant < 0.0) {
            return float3(0.0);
        }
        float signed_root = qb >= 0.0 ? sqrt(discriminant) : -sqrt(discriminant);
        float numerator = -0.5 * (qb + signed_root);
        roots = float2(numerator / qa);
        if (numerator != 0.0) {
            roots.y = qc / numerator;
        }
    }
    float3 answer = float3(0.0);
    for (uint index = 0; index < 2; index += 1) {
        float v = roots[index];
        if (!(v >= 0.0 && v <= 1.0)) {
            continue;
        }
        float2 direction = horizontal + v * bend;
        float2 remainder = delta - v * vertical;
        float u;
        if (abs(direction.x) >= abs(direction.y)) {
            if (direction.x == 0.0) {
                continue;
            }
            u = remainder.x / direction.x;
        } else {
            u = remainder.y / direction.y;
        }
        if (!(u >= 0.0 && u <= 1.0)) {
            continue;
        }
        if (mesh_cross(direction, vertical + u * bend) == 0.0) {
            continue;
        }
        if (answer.z == 0.0 || v > answer.y || (v == answer.y && u > answer.x)) {
            answer = float3(u, v, 1.0);
        }
    }
    return answer;
}

// The premultiplied colour the mesh gives `point`; transparent outside
// every patch.
static float4 mesh_color(
    float2 point,
    constant MeshUniforms& uniforms,
    constant MeshPatch* patches) {
    for (uint remaining = uniforms.count; remaining > 0; remaining -= 1) {
        constant MeshPatch& patch = patches[remaining - 1];
        float3 uv = mesh_uv(point, patch.top, patch.bottom);
        if (uv.z != 0.0) {
            float2 weight = uv.xy;
            if (uniforms.smooth_color != 0) {
                weight = weight * weight * (3.0 - 2.0 * weight);
            }
            float4 top = mix(patch.colors[0], patch.colors[1], weight.x);
            float4 bottom = mix(patch.colors[2], patch.colors[3], weight.x);
            return mix(top, bottom, weight.y);
        }
    }
    return float4(0.0);
}

// The sub-pixel points a pixel averages, as offsets from its centre in
// pixels: `SAMPLE_OFFSETS` in patch.rs, eight rooks.
constant float2 sample_offsets[8] = {
    float2(0.1875, -0.4375),
    float2(-0.1875, -0.3125),
    float2(0.4375, -0.1875),
    float2(-0.4375, -0.0625),
    float2(-0.0625, 0.0625),
    float2(0.3125, 0.1875),
    float2(-0.3125, 0.3125),
    float2(0.0625, 0.4375),
};

// A pixel's colour is the mean of the mesh over its sample points, so a
// pixel the boundary of the patches crosses is covered in proportion. The
// mesh point moves affinely with the pixel, so its screen-space
// derivatives are the step of one pixel in the mesh's space.
fragment half4 mesh_gradient_fragment(
    MeshVertexOut input [[stage_in]],
    constant MeshUniforms& uniforms [[buffer(0)]],
    constant MeshPatch* patches [[buffer(1)]]) {
    float2 step_x = dfdx(input.point);
    float2 step_y = dfdy(input.point);
    float4 sum = float4(0.0);
    for (uint index = 0; index < 8; index += 1) {
        float2 offset = sample_offsets[index];
        sum += mesh_color(
            input.point + step_x * offset.x + step_y * offset.y,
            uniforms,
            patches);
    }
    return half4(sum * 0.125);
}
