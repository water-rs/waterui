// Resolves the flooded field into the shape's signed distance field, in
// pixels and negative inside, in the red channel.
//
// A pixel no contour point reached belongs to an input with no contour:
// nothing is covered, so it is farther from the shape than any distance the
// Rgba16Float field holds, and stores the largest one.

@group(0) @binding(0) var field: texture_2d<f32>;

// The largest finite f16, the field's texel precision.
const FARTHEST: f32 = 65504.0;

@fragment
fn main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(field, vec2<i32>(position.xy), 0);
    if texel.z < 0.5 {
        return vec4<f32>(FARTHEST, 0.0, 0.0, 1.0);
    }
    let magnitude = length(texel.xy);
    return vec4<f32>(select(magnitude, -magnitude, texel.w > 0.5), 0.0, 0.0, 1.0);
}
