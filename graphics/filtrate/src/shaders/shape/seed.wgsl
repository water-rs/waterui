// Seeds the shape's distance field from the input's coverage.
//
// The shape is where the input's alpha exceeds one half. The iso-contour
// alpha = 0.5 passes between two 4-neighbours on opposite sides of it, at
// the point linear interpolation of their coverage puts it. Every pixel
// with such a neighbour records the offset from its centre to the nearest
// point of the contour, taken as the straight line through that crossing
// whose normal is the coverage gradient. Texels outside the input are
// uncovered, so content reaching an edge of the input ends there.
//
// Output: xy = offset to the contour in pixels, z = 1 when xy holds one,
// w = 1 when the pixel is inside the shape.

@group(0) @binding(0) var coverage: texture_2d<f32>;

fn alpha_at(pixel: vec2<i32>, size: vec2<i32>) -> f32 {
    if any(pixel < vec2<i32>(0)) || any(pixel >= size) {
        return 0.0;
    }
    return textureLoad(coverage, pixel, 0).a;
}

@fragment
fn main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(coverage));
    let pixel = vec2<i32>(position.xy);
    let alpha = alpha_at(pixel, size);
    let inside = alpha > 0.5;
    let flag = select(0.0, 1.0, inside);

    let directions = array<vec2<i32>, 4>(
        vec2<i32>(-1, 0),
        vec2<i32>(1, 0),
        vec2<i32>(0, -1),
        vec2<i32>(0, 1),
    );
    var neighbours: array<f32, 4>;
    var nearest = 2.0;
    var crossing = vec2<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let other = alpha_at(pixel + directions[i], size);
        neighbours[i] = other;
        if (other > 0.5) != inside {
            // `inside` differs, so `alpha - other` is non-zero.
            let t = (alpha - 0.5) / (alpha - other);
            if t < nearest {
                nearest = t;
                crossing = vec2<f32>(directions[i]) * t;
            }
        }
    }
    if nearest > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, flag);
    }

    let gradient = vec2<f32>(neighbours[1] - neighbours[0], neighbours[3] - neighbours[2]);
    let magnitude = length(gradient);
    var offset = crossing;
    if magnitude > 0.0 {
        let normal = gradient / magnitude;
        offset = normal * dot(crossing, normal);
    }
    return vec4<f32>(offset, 1.0, flag);
}
