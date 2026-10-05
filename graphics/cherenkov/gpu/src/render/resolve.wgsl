// A backdrop group's reduced-scale capture: the downsampling resolve of
// its region into the capture grid. At scale `s` the grid is anchored at
// the device origin and texel `i` covers the device interval
// `[i/s, (i+1)/s)` per axis, clipped to the backdrop's extent; its value
// is the area-weighted mean of the device pixels under it, premultiplied
// extended linear Display P3, never clamped.

// The pass's slot in the per-pass globals buffer: the engine `Globals`
// (unused here) followed by the resolve parameters.
struct Resolve {
    size: vec2<f32>,
    origin: vec2<f32>,
    space: u32,
    pad: u32,
    attachment_origin: vec2<f32>,
    // The capture region's origin on the capture grid, in texels.
    texel_origin: vec2<f32>,
    // The device position of the source texture's texel (0, 0).
    source_origin: vec2<f32>,
    // The device extent of the backdrop: spans clip to `[0, extent)`.
    extent: vec2<f32>,
    // The capture scale `s`.
    scale: f32,
    pad2: f32,
}

@group(0) @binding(0) var<uniform> params: Resolve;
@group(0) @binding(1) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let i = floor(position.xy) + params.texel_origin;
    let lo = i / params.scale;
    let hi = min((i + 1.0) / params.scale, params.extent);
    let first = vec2<i32>(floor(lo));
    let last = vec2<i32>(ceil(hi));
    let origin = vec2<i32>(params.source_origin);
    var acc = vec4<f32>(0.0);
    for (var y = first.y; y < last.y; y += 1) {
        let fy = f32(y);
        let wy = min(hi.y, fy + 1.0) - max(lo.y, fy);
        var row = vec4<f32>(0.0);
        for (var x = first.x; x < last.x; x += 1) {
            let fx = f32(x);
            let wx = min(hi.x, fx + 1.0) - max(lo.x, fx);
            row += textureLoad(source, vec2<i32>(x, y) - origin, 0) * wx;
        }
        acc += row * wy;
    }
    let span = hi - lo;
    return acc / (span.x * span.y);
}
