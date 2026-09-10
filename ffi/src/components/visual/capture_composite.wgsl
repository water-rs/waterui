// Draws one rendered GPU surface into a captured view subtree.
//
// Android's HWUI records a SurfaceView as a cleared hole, so a GpuSurface
// inside a filtered subtree is missing from the captured buffer and is drawn
// back into it here, at the rectangle the surface occupies.
//
// The destination rectangle is carried per draw rather than set as a viewport,
// so a surface that hangs over the edge of the capture is clipped by the
// rasterizer instead of being invalid.

struct Placement {
    // Destination rectangle in target pixels: origin in `xy`, size in `zw`.
    rect: vec4<f32>,
    // The capture texture's own size in pixels.
    // Named for what it holds: `target` alone is a WGSL reserved keyword.
    target_size: vec2<f32>,
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;
@group(0) @binding(2) var<uniform> placement: Placement;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // The unit square as two triangles; the same value indexes the destination
    // rectangle and the source texture, both of which have their origin at the
    // top left.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );

    let corner = corners[vertex_index];
    let pixel = placement.rect.xy + corner * placement.rect.zw;

    var output: VertexOutput;
    output.position = vec4<f32>(
        pixel.x / placement.target_size.x * 2.0 - 1.0,
        1.0 - pixel.y / placement.target_size.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = corner;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // An explicit mip level keeps this valid for unfilterable float formats,
    // which the HDR capture path uses.
    return textureSampleLevel(t_source, s_source, input.uv, 0.0);
}
