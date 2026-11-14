// One mip level of a projective layer's local image (#84): the area
// average of the previous level in premultiplied extended linear Display
// P3, never clamped. Each level is `max(1, floor(previous / 2))` texels
// per axis — the hardware chain — and spans the same source extent, so
// destination texel `i` covers the source interval `[i·s, (i+1)·s)`,
// `s = previous / current ∈ [2, 3]`: a 2×2 box for even sizes,
// area-overlap weights over up to three texels per axis for odd ones.

@group(0) @binding(0) var previous: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

// The overlap of destination texel `i`'s source interval with source
// texel `j`, normalized by the interval length.
fn overlap(i: f32, j: f32, s: f32) -> f32 {
    let lo = i * s;
    let hi = (i + 1.0) * s;
    return max(min(hi, j + 1.0) - max(lo, j), 0.0) / s;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let src = vec2<f32>(textureDimensions(previous));
    let dst = max(floor(src * 0.5), vec2<f32>(1.0));
    let s = src / dst;
    let i = floor(position.xy);
    let first = floor(i * s);
    var acc = vec4<f32>(0.0);
    for (var y = 0; y < 3; y += 1) {
        let jy = first.y + f32(y);
        let wy = overlap(i.y, jy, s.y);
        if jy >= src.y || wy <= 0.0 {
            continue;
        }
        for (var x = 0; x < 3; x += 1) {
            let jx = first.x + f32(x);
            let wx = overlap(i.x, jx, s.x);
            if jx >= src.x || wx <= 0.0 {
                continue;
            }
            acc += textureLoad(previous, vec2<i32>(i32(jx), i32(jy)), 0) * (wx * wy);
        }
    }
    return acc;
}
