// One jump-flooding step of the shape's distance field: every pixel takes
// the nearest contour point among its own and those of the eight pixels
// `jump` away.
//
// Texel layout as the seed pass writes it: xy = offset to the contour in
// pixels, z = 1 when xy holds one, w = 1 when the pixel is inside.

@group(0) @binding(0) var field: texture_2d<f32>;
// x: the jump distance in pixels.
@group(0) @binding(1) var<uniform> jump: vec4<i32>;

@fragment
fn main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(field));
    let pixel = vec2<i32>(position.xy);
    let own = textureLoad(field, pixel, 0);
    var found = own.z > 0.5;
    var best = own.xy;
    var best_distance = dot(own.xy, own.xy);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let hop = vec2<i32>(x, y) * jump.x;
            let other = pixel + hop;
            if (x == 0 && y == 0) || any(other < vec2<i32>(0)) || any(other >= size) {
                continue;
            }
            let texel = textureLoad(field, other, 0);
            if texel.z < 0.5 {
                continue;
            }
            let offset = vec2<f32>(hop) + texel.xy;
            let squared = dot(offset, offset);
            if !found || squared < best_distance {
                found = true;
                best = offset;
                best_distance = squared;
            }
        }
    }
    return vec4<f32>(best, select(0.0, 1.0, found), own.w);
}
