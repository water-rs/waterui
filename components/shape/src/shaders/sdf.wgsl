struct Uniforms {
    color: vec4<f32>,
    shape_type: u32,
    dimensions: vec2<f32>,
    radii: vec4<f32>, // tl, tr, br, bl
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
    out.uv.y = 1.0 - out.uv.y; // Make 0,0 top-left
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
    // Select radius for the quadrant
    // Radii: x=TL, y=TR, z=BR, w=BL
    let radius_val = select(
        select(r.x, r.y, p.x > 0.0), // p.y < 0: Top
        select(r.w, r.z, p.x > 0.0), // p.y > 0: Bottom
        p.y > 0.0
    );
    
    let q = abs(p) - b + vec2<f32>(radius_val);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius_val;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = uniforms.dimensions;
    let center = size * 0.5;
    let p = (in.uv * size) - center;
    
    var dist: f32 = 0.0;
    
    if (uniforms.shape_type == 0u) { // Rect
        dist = sd_rect(p, size * 0.5);
    } else if (uniforms.shape_type == 1u) { // Circle
        let r = min(size.x, size.y) * 0.5;
        dist = sd_circle(p, r);
    } else if (uniforms.shape_type == 2u) { // Ellipse
        let semi = size * 0.5;
        let val = length(p / semi) - 1.0;
        let w = fwidth(val); 
        let alpha = 1.0 - smoothstep(-w, w, val);
        return vec4<f32>(uniforms.color.rgb, uniforms.color.a * alpha);
    } else if (uniforms.shape_type == 3u) { // RoundedRect
        let min_dim = min(size.x, size.y);
        let r_vec = uniforms.radii * min_dim;
        dist = sd_rounded_box(p, size * 0.5, r_vec);
    } else if (uniforms.shape_type == 4u) { // Capsule
        let r = min(size.x, size.y) * 0.5;
        dist = sd_rounded_box(p, size * 0.5, vec4<f32>(r));
    }
    
    let alpha = 1.0 - smoothstep(-0.5, 0.5, dist);
    return vec4<f32>(uniforms.color.rgb, uniforms.color.a * alpha);
}
