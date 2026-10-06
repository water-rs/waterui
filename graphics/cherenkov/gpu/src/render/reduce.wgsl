// A backdrop group's capture pyramid: level `k` texel `(i, j)` is the
// mean of level `k − 1` texels `(2i..=2i+1, 2j..=2j+1)` — an exact 2×2
// box reduction matching the capture's area-weighted resolve. A partial
// box at the grid's edge averages the texels present: the clamped reads
// duplicate the edge texel, so the four-tap mean is exactly that average,
// values unclamped.

// The pass's slot in the per-pass globals buffer: the engine `Globals`
// (unused here) followed by the reduce parameters.
struct Reduce {
    size: vec2<f32>,
    origin: vec2<f32>,
    space: u32,
    pad: u32,
    attachment_origin: vec2<f32>,
    // The source level's spec extent, in its own texels.
    source_extent: vec2<f32>,
    pad0: vec2<f32>,
    pad1: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Reduce;
@group(0) @binding(1) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let d = vec2<i32>(floor(position.xy));
    let extent = vec2<i32>(params.source_extent);
    let lo = min(2 * d, extent - 1);
    let hi = min(2 * d + 1, extent - 1);
    let c00 = textureLoad(source, lo, 0);
    let c10 = textureLoad(source, vec2<i32>(hi.x, lo.y), 0);
    let c01 = textureLoad(source, vec2<i32>(lo.x, hi.y), 0);
    let c11 = textureLoad(source, hi, 0);
    return (c00 + c10 + c01 + c11) * 0.25;
}
