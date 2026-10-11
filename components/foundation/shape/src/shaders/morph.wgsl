// Morph-shape fragment, written against the Cherenkov shader-paint prelude:
// `uniforms.time`/`uniforms.resolution` and `in.uv` come from the engine, and
// `params` is the `ShaderPaint::uniforms` list — colour, progress, shape
// types and the two radius sets, in order.

fn sd_rect(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(p) - b;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

fn sd_circle(p: vec2<f32>, r: f32) -> f32 {
    return length(p) - r;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: vec4<f32>) -> f32 {
    // Radii are ordered TL, TR, BR, BL.
    let radius_val = select(
        select(r.x, r.y, p.x > 0.0),
        select(r.w, r.z, p.x > 0.0),
        p.y > 0.0
    );
    let q = abs(p) - b + vec2<f32>(radius_val);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius_val;
}

fn shape_distance(shape_type: u32, p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (shape_type == 0u) { // Rect
        return sd_rect(p, size * 0.5);
    }
    if (shape_type == 1u) { // Circle
        return sd_circle(p, min(size.x, size.y) * 0.5);
    }
    if (shape_type == 2u) { // Ellipse
        let semi = size * 0.5;
        return (length(p / semi) - 1.0) * min(semi.x, semi.y);
    }
    if (shape_type == 3u) { // RoundedRect
        let min_dim = min(size.x, size.y);
        return sd_rounded_box(p, size * 0.5, radii * min_dim);
    }
    if (shape_type == 4u) { // Capsule
        let r = min(size.x, size.y) * 0.5;
        return sd_rounded_box(p, size * 0.5, vec4<f32>(r));
    }
    return sd_rect(p, size * 0.5);
}

@fragment
fn main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = params[0];
    let size = uniforms.resolution;
    let progress = clamp(params[1].x, 0.0, 1.0);
    let from_shape = u32(params[2].x + 0.5);
    let to_shape = u32(params[2].y + 0.5);
    let p = (in.uv * size) - (size * 0.5);

    let from_dist = shape_distance(from_shape, p, size, params[3]);
    let to_dist = shape_distance(to_shape, p, size, params[4]);
    let dist = mix(from_dist, to_dist, progress);

    // Pixel-accurate edge smoothing derived from signed-distance derivatives.
    let aa = max(fwidth(dist), 0.5);
    let alpha = 1.0 - smoothstep(-aa, aa, dist);
    // `color` is premultiplied; scaling it by coverage keeps the paint in the
    // engine's premultiplied output convention.
    return color * alpha;
}
