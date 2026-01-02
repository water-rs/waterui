// Line chart shader.
//
// Renders line segments using instanced rendering. Each instance is a segment
// between two adjacent points, drawn as a quad (2 triangles).
//
// Supports:
// - Animation interpolation between previous and current data
// - Entry animation (line draws from left to right)
// - Area fill below the line
// - Hover highlighting

// Chart uniforms (same layout as bar chart)
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, 0] - normalized coordinates, -1 if not hovering
    pointer: vec4<f32>,
}

// Line-specific uniforms
struct LineUniforms {
    chart: ChartUniforms,
    line_color: vec4<f32>,
    line_width: f32,
    show_fill: f32,
    fill_opacity: f32,
    point_count: f32,
}

// Data point
struct LinePoint {
    x: f32,
    y: f32,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: LineUniforms;
@group(0) @binding(1) var<storage, read> current_data: array<LinePoint>;
@group(0) @binding(2) var<storage, read> previous_data: array<LinePoint>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,           // UV for anti-aliasing
    @location(2) segment_index: f32,      // Which segment this is
    @location(3) is_fill: f32,            // 1.0 if this is area fill, 0.0 if line
}

// Note: Easing functions provided by common.wgsl (prepended at compile time)

// Normalize Y value to [0, 1] based on data bounds
fn normalize_y(y: f32) -> f32 {
    let y_range = uniforms.chart.bounds.w - uniforms.chart.bounds.z;
    return select(
        (y - uniforms.chart.bounds.z) / y_range,
        0.0,
        y_range <= 0.0
    );
}

// Normalize X value to [0, 1] based on data bounds
fn normalize_x(x: f32) -> f32 {
    let x_range = uniforms.chart.bounds.y - uniforms.chart.bounds.x;
    return select(
        (x - uniforms.chart.bounds.x) / x_range,
        0.0,
        x_range <= 0.0
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let point_count = arrayLength(&current_data);
    let segment_count = point_count - 1u;

    if instance_index >= segment_count {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get current and previous points for this segment
    let curr_p0 = current_data[instance_index];
    let curr_p1 = current_data[instance_index + 1u];
    let prev_p0 = previous_data[instance_index];
    let prev_p1 = previous_data[instance_index + 1u];

    // Apply animation interpolation
    let progress = clamp(uniforms.chart.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.chart.animation.z));

    var x0 = mix(prev_p0.x, curr_p0.x, eased_progress);
    var y0 = mix(prev_p0.y, curr_p0.y, eased_progress);
    var x1 = mix(prev_p1.x, curr_p1.x, eased_progress);
    var y1 = mix(prev_p1.y, curr_p1.y, eased_progress);

    // Normalize to chart coordinates
    let norm_x0 = normalize_x(x0);
    let norm_y0 = normalize_y(y0);
    let norm_x1 = normalize_x(x1);
    let norm_y1 = normalize_y(y1);

    // Entry animation: draw from left to right
    if uniforms.chart.animation.w > 0.5 {
        let segment_progress = f32(instance_index) / f32(segment_count);
        if segment_progress > eased_progress {
            // This segment hasn't started drawing yet
            out.position = vec4<f32>(0.0);
            return out;
        }
    }

    // Calculate line direction and perpendicular
    let dir = vec2<f32>(norm_x1 - norm_x0, norm_y1 - norm_y0);
    let len = length(dir);
    let norm_dir = select(vec2<f32>(1.0, 0.0), dir / len, len > 0.0001);
    let perp = vec2<f32>(-norm_dir.y, norm_dir.x);

    // Line width in normalized coordinates
    let half_width = uniforms.line_width * 0.5 / uniforms.chart.viewport.x;

    // Generate quad vertices
    // Vertices 0-2: first triangle, 3-5: second triangle
    var local_pos: vec2<f32>;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { local_pos = vec2(norm_x0, norm_y0) + perp * half_width; uv = vec2(0.0, 1.0); }
        case 1u: { local_pos = vec2(norm_x0, norm_y0) - perp * half_width; uv = vec2(0.0, 0.0); }
        case 2u: { local_pos = vec2(norm_x1, norm_y1) + perp * half_width; uv = vec2(1.0, 1.0); }
        case 3u: { local_pos = vec2(norm_x0, norm_y0) - perp * half_width; uv = vec2(0.0, 0.0); }
        case 4u: { local_pos = vec2(norm_x1, norm_y1) - perp * half_width; uv = vec2(1.0, 0.0); }
        case 5u: { local_pos = vec2(norm_x1, norm_y1) + perp * half_width; uv = vec2(1.0, 1.0); }
        default: { local_pos = vec2(0.0); uv = vec2(0.0); }
    }

    // Map to NDC with padding
    let padding = 0.1;
    let ndc_x = (local_pos.x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (local_pos.y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = uniforms.line_color;
    out.uv = uv;
    out.segment_index = f32(instance_index);
    out.is_fill = 0.0;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // SDF line: distance from center line (0 at center, 1 at edge)
    // UV.y = 0 at bottom edge, 0.5 at center, 1 at top edge
    let edge_dist = abs(in.uv.y - 0.5) * 2.0 - 1.0;  // SDF: 0 at edge, negative inside

    // Anti-aliased coverage using fwidth for resolution-independent AA
    let aa = sdf_coverage(edge_dist);

    // Early discard for fully transparent pixels
    if aa < 0.001 {
        discard;
    }

    // Check if pointer is near this segment for hover effect
    if uniforms.chart.pointer.x >= 0.0 {
        let segment_count = u32(uniforms.point_count) - 1u;
        let segment_start = in.segment_index / f32(segment_count);
        let segment_end = (in.segment_index + 1.0) / f32(segment_count);

        let pointer_x = uniforms.chart.pointer.x;
        if pointer_x >= segment_start && pointer_x <= segment_end {
            // Highlight on hover
            color = vec4<f32>(
                min(color.r * 1.3, 1.0),
                min(color.g * 1.3, 1.0),
                min(color.b * 1.3, 1.0),
                color.a
            );
        }
    }

    // Apply alpha and anti-aliasing
    let final_alpha = color.a * aa;

    // Premultiply alpha for correct blending
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
