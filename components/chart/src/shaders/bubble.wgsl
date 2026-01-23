// Bubble chart shader.
//
// Renders scatter points with variable sizes (radii).
// Each bubble is rendered as a circle with anti-aliased edges.

struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Style: [default_color.rgba as vec4]
    default_color: vec4<f32>,
    // Size: [min_radius, max_radius, size_min, size_max]
    size_config: vec4<f32>,
}

struct BubblePoint {
    x: f32,
    y: f32,
    size: f32,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<storage, read> points: array<BubblePoint>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) point_index: u32,
}

// Convert data coordinates to padded NDC
fn data_to_padded_ndc(x: f32, y: f32) -> vec2<f32> {
    let padding = 0.1;
    let norm_x = (x - uniforms.bounds.x) / (uniforms.bounds.y - uniforms.bounds.x);
    let norm_y = (y - uniforms.bounds.z) / (uniforms.bounds.w - uniforms.bounds.z);
    let ndc_x = (norm_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (norm_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    return vec2<f32>(ndc_x, ndc_y);
}

// Map size value to radius
fn size_to_radius(size: f32) -> f32 {
    let size_min = uniforms.size_config.z;
    let size_max = uniforms.size_config.w;
    let radius_min = uniforms.size_config.x;
    let radius_max = uniforms.size_config.y;

    let t = clamp((size - size_min) / max(size_max - size_min, 0.001), 0.0, 1.0);
    return mix(radius_min, radius_max, sqrt(t)); // sqrt for area-proportional sizing
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let point = points[instance_index];

    // Animation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));
    let anim_scale = select(1.0, eased_progress, uniforms.animation.w > 0.5);

    // Get center position in NDC
    let center = data_to_padded_ndc(point.x, point.y);

    // Calculate radius in NDC space
    let radius_pixels = size_to_radius(point.size) * anim_scale;
    let radius_ndc = vec2<f32>(
        radius_pixels * 2.0 / uniforms.viewport.x,
        radius_pixels * 2.0 / uniforms.viewport.y
    );

    // Generate quad vertices (6 vertices per bubble)
    var offset: vec2<f32>;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { offset = vec2<f32>(-1.0, -1.0); uv = vec2<f32>(0.0, 0.0); }
        case 1u: { offset = vec2<f32>(-1.0,  1.0); uv = vec2<f32>(0.0, 1.0); }
        case 2u: { offset = vec2<f32>( 1.0,  1.0); uv = vec2<f32>(1.0, 1.0); }
        case 3u: { offset = vec2<f32>(-1.0, -1.0); uv = vec2<f32>(0.0, 0.0); }
        case 4u: { offset = vec2<f32>( 1.0,  1.0); uv = vec2<f32>(1.0, 1.0); }
        case 5u: { offset = vec2<f32>( 1.0, -1.0); uv = vec2<f32>(1.0, 0.0); }
        default: { offset = vec2<f32>(0.0); uv = vec2<f32>(0.0); }
    }

    let pos = center + offset * radius_ndc;

    // Use point color if alpha > 0, otherwise use default
    var color = uniforms.default_color;
    if point.color.a > 0.0 {
        color = point.color;
    }

    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.color = color;
    out.uv = uv;
    out.point_index = instance_index;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Distance from center (uv is 0-1, center is 0.5)
    let centered_uv = in.uv * 2.0 - 1.0;
    let dist = length(centered_uv);

    // Anti-aliased edge using SDF + fwidth
    let edge_dist = dist - 1.0;
    let alpha = sdf_coverage(edge_dist);
    if alpha < 0.001 {
        discard;
    }

    var color = in.color;
    color.a *= alpha;

    // Add subtle gradient for 3D effect
    let highlight = 1.0 - dist * 0.3;
    color = vec4<f32>(color.rgb * highlight, color.a);

    // Premultiply alpha
    return vec4<f32>(color.rgb * color.a, color.a);
}
