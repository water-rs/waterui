// Source headroom of an immutable capture. Each output texel contains
// the maximum straight working-space component in a 16 by 16 source tile.
// Repeated reductions end at one texel; no display headroom is involved.
// `fs_main` reads working-space texels; `fs_decode` reads a capture stored
// in premultiplied encoded sRGB and moves each texel to the working space
// through the space fragment (`space_from_srgb`) that precedes this file,
// for the first reduction of a surface storing that space.
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}

// The peak over the tile at `position`, each texel decoded from encoded
// sRGB first when `decode` holds.
fn tile_peak(position: vec4<f32>, decode: bool) -> vec4<f32> {
    let origin = vec2<u32>(position.xy) * 16u;
    let end = min(origin + vec2<u32>(16u), textureDimensions(source));
    var peak = 1.0;
    for (var y = origin.y; y < end.y; y++) {
        for (var x = origin.x; x < end.x; x++) {
            var color = textureLoad(source, vec2<i32>(i32(x), i32(y)), 0);
            if decode {
                color = space_from_srgb(color);
            }
            if color.a > 0.0 {
                peak = max(peak, max(color.r, max(color.g, color.b)) / color.a);
            }
        }
    }
    return vec4<f32>(peak, peak, peak, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return tile_peak(position, false);
}

@fragment
fn fs_decode(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return tile_peak(position, true);
}
