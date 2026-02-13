struct Uniforms {
    color: vec4<f32>,
    dimensions_and_progress: vec4<f32>, // width, height, progress, _pad
    shape_types: vec4<f32>,             // from_type, to_type, _pad, _pad
    from_radii: vec4<f32>,              // tl, tr, br, bl
    to_radii: vec4<f32>,                // tl, tr, br, bl
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );
    let pos = positions[vertex_index];

    var out: VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + 0.5;
    out.uv.y = 1.0 - out.uv.y;
    return out;
}

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
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = uniforms.dimensions_and_progress.xy;
    let progress = clamp(uniforms.dimensions_and_progress.z, 0.0, 1.0);
    let from_shape = u32(uniforms.shape_types.x + 0.5);
    let to_shape = u32(uniforms.shape_types.y + 0.5);
    let p = (in.uv * size) - (size * 0.5);

    let from_dist = shape_distance(from_shape, p, size, uniforms.from_radii);
    let to_dist = shape_distance(to_shape, p, size, uniforms.to_radii);
    let dist = mix(from_dist, to_dist, progress);

    // Pixel-accurate edge smoothing derived from signed-distance derivatives.
    let aa = max(fwidth(dist), 0.5);
    let alpha = 1.0 - smoothstep(-aa, aa, dist);
    return vec4<f32>(uniforms.color.rgb, uniforms.color.a * alpha);
}
