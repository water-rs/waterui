// Stacked area chart shader.
//
// Renders stacked area charts using instanced rendering. Each instance is a
// segment between two adjacent X values for one series, drawn as a quad.
//
// Supports:
// - Multiple stacked series with cumulative baselines
// - Animation interpolation between previous and current data
// - Semi-transparent fills with proper blending

// Chart uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, 0]
    pointer: vec4<f32>,
}

// Area-specific uniforms
struct AreaUniforms {
    chart: ChartUniforms,
    point_count: u32,
    series_count: u32,
    stacked: u32,
    _pad: u32,
}

// Per-segment data (4 floats per segment per series)
// [x0, y0_top, y0_bottom, series_index] for first point
// [x1, y1_top, y1_bottom, unused] for second point
struct AreaSegment {
    x0: f32,
    y0_top: f32,
    y0_bottom: f32,
    series_index: f32,
    x1: f32,
    y1_top: f32,
    y1_bottom: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> uniforms: AreaUniforms;
@group(0) @binding(1) var<storage, read> segments: array<AreaSegment>;
@group(0) @binding(2) var<storage, read> colors: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> prev_segments: array<AreaSegment>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
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

    let segment_count = arrayLength(&segments);
    if instance_index >= segment_count {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get current and previous segment data
    let curr = segments[instance_index];
    let prev = prev_segments[instance_index];

    // Get series color
    let series_idx = u32(curr.series_index);
    out.color = colors[series_idx];

    // Apply animation interpolation
    let progress = clamp(uniforms.chart.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.chart.animation.z));

    let x0 = mix(prev.x0, curr.x0, eased_progress);
    let y0_top = mix(prev.y0_top, curr.y0_top, eased_progress);
    let y0_bottom = mix(prev.y0_bottom, curr.y0_bottom, eased_progress);
    let x1 = mix(prev.x1, curr.x1, eased_progress);
    let y1_top = mix(prev.y1_top, curr.y1_top, eased_progress);
    let y1_bottom = mix(prev.y1_bottom, curr.y1_bottom, eased_progress);

    // Normalize coordinates
    let norm_x0 = normalize_x(x0);
    let norm_x1 = normalize_x(x1);
    let norm_y0_top = normalize_y(y0_top);
    let norm_y0_bottom = normalize_y(y0_bottom);
    let norm_y1_top = normalize_y(y1_top);
    let norm_y1_bottom = normalize_y(y1_bottom);

    // Entry animation: draw from left to right
    if uniforms.chart.animation.w > 0.5 {
        let point_progress = norm_x0;
        if point_progress > eased_progress {
            out.position = vec4<f32>(0.0);
            return out;
        }
    }

    // Generate quad vertices (6 per segment: 2 triangles)
    // Quad spans from (x0, y0_bottom) to (x1, y1_top)
    var local_pos: vec2<f32>;

    switch vertex_index {
        // Triangle 1: bottom-left, bottom-right, top-left
        case 0u: { local_pos = vec2(norm_x0, norm_y0_bottom); out.uv = vec2(0.0, 0.0); }
        case 1u: { local_pos = vec2(norm_x1, norm_y1_bottom); out.uv = vec2(1.0, 0.0); }
        case 2u: { local_pos = vec2(norm_x0, norm_y0_top); out.uv = vec2(0.0, 1.0); }
        // Triangle 2: bottom-right, top-right, top-left
        case 3u: { local_pos = vec2(norm_x1, norm_y1_bottom); out.uv = vec2(1.0, 0.0); }
        case 4u: { local_pos = vec2(norm_x1, norm_y1_top); out.uv = vec2(1.0, 1.0); }
        case 5u: { local_pos = vec2(norm_x0, norm_y0_top); out.uv = vec2(0.0, 1.0); }
        default: { local_pos = vec2(0.0); out.uv = vec2(0.0); }
    }

    // Map to NDC with padding
    let padding = 0.1;
    let ndc_x = (local_pos.x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (local_pos.y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // Check for hover highlighting
    if uniforms.chart.pointer.x >= 0.0 {
        let hover_x = uniforms.chart.pointer.x;
        let hover_y = uniforms.chart.pointer.y;

        // Simple hover detection based on UV proximity
        let dist_x = abs(in.uv.x - 0.5);
        if dist_x < 0.3 {
            // Subtle highlight when hovering over this segment
            color = vec4<f32>(
                min(color.r * 1.15, 1.0),
                min(color.g * 1.15, 1.0),
                min(color.b * 1.15, 1.0),
                color.a
            );
        }
    }

    // Premultiply alpha for correct blending
    return vec4<f32>(color.rgb * color.a, color.a);
}
