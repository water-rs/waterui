// Bar chart shader.
//
// Renders bars using instanced rendering. Each instance is a bar,
// with 6 vertices forming 2 triangles (a quad).
//
// Supports:
// - Animation interpolation between previous and current data
// - Entry animation (bars grow from bottom)
// - Hover highlighting

// Uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, data_count] - normalized coordinates, -1 if not hovering
    pointer: vec4<f32>,
}

// Data point with color
struct DataPoint {
    x: f32,
    y: f32,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<storage, read> current_data: array<DataPoint>;
@group(0) @binding(2) var<storage, read> previous_data: array<DataPoint>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) bar_uv: vec2<f32>,      // UV within the bar (for effects)
    @location(2) bar_index: f32,         // Which bar this is
}

// Note: Easing functions provided by common.wgsl (prepended at compile time)

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let data_count = max(u32(uniforms.pointer.w), 1u);
    if instance_index >= data_count {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get current and previous data points
    let curr = current_data[instance_index];
    let prev = previous_data[instance_index];

    // Apply animation interpolation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    let x = mix(prev.x, curr.x, eased_progress);
    var y = mix(prev.y, curr.y, eased_progress);
    let color = mix(prev.color, curr.color, eased_progress);

    // Entry animation: grow from bottom
    if uniforms.animation.w > 0.5 {
        y = y * eased_progress;
    }

    // Calculate bar dimensions
    let bar_count = max(uniforms.pointer.w, 1.0);
    let bar_width = 0.8 / bar_count;
    let x_range = max(uniforms.bounds.y - uniforms.bounds.x, 1e-6);

    // Calculate bar position from data X (aligned to axis ticks)
    let normalized_x = (x - uniforms.bounds.x) / x_range;
    let bar_center_x = clamp(normalized_x, 0.0, 1.0);

    // Normalize Y to [0, 1] based on data bounds
    let y_range = uniforms.bounds.w - uniforms.bounds.z;
    let normalized_y = select(
        (y - uniforms.bounds.z) / y_range,
        0.0,
        y_range <= 0.0
    );

    // Generate quad vertices (6 vertices = 2 triangles)
    // Vertex order: 0-1-2 (bottom-left, top-left, top-right), 0-2-3 (bottom-left, top-right, bottom-right)
    var local_x: f32;
    var local_y: f32;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { local_x = -0.5; local_y = 0.0; uv = vec2(0.0, 0.0); }
        case 1u: { local_x = -0.5; local_y = 1.0; uv = vec2(0.0, 1.0); }
        case 2u: { local_x = 0.5;  local_y = 1.0; uv = vec2(1.0, 1.0); }
        case 3u: { local_x = -0.5; local_y = 0.0; uv = vec2(0.0, 0.0); }
        case 4u: { local_x = 0.5;  local_y = 1.0; uv = vec2(1.0, 1.0); }
        case 5u: { local_x = 0.5;  local_y = 0.0; uv = vec2(1.0, 0.0); }
        default: { local_x = 0.0; local_y = 0.0; uv = vec2(0.0); }
    }

    // Transform to normalized device coordinates [-1, 1]
    // Chart area with padding (10% on each side)
    let padding = 0.1;
    let chart_x = bar_center_x + local_x * bar_width;
    let chart_y = local_y * normalized_y;

    // Map to NDC
    let ndc_x = (chart_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (chart_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = color;
    out.bar_uv = uv;
    out.bar_index = f32(instance_index);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // SDF for rounded rectangle edges (bar with rounded corners)
    // UV is [0,1] within bar, convert to centered coordinates
    let centered = (in.bar_uv - 0.5) * 2.0;  // [-1, 1]
    let corner_radius = 0.1;  // 10% of bar dimension
    let dist = sdf_rounded_rect(centered, vec2<f32>(1.0 - corner_radius, 1.0 - corner_radius), corner_radius);

    // Anti-aliased coverage using fwidth for resolution-independent AA
    let aa = sdf_coverage(dist);

    // Early discard for fully transparent pixels
    if aa < 0.001 {
        discard;
    }

    // Check if this bar is being hovered
    if uniforms.pointer.x >= 0.0 {
        let data_count = max(uniforms.pointer.w, 1.0);
        let bar_width = 0.8 / data_count;
        let x_range = max(uniforms.bounds.y - uniforms.bounds.x, 1e-6);
        let pointer_x = uniforms.pointer.x;
        let bar_center_x = clamp((current_data[u32(in.bar_index)].x - uniforms.bounds.x) / x_range, 0.0, 1.0);

        let bar_left = bar_center_x - bar_width * 0.5;
        let bar_right = bar_center_x + bar_width * 0.5;

        if pointer_x >= bar_left && pointer_x <= bar_right {
            // Highlight on hover
            color = vec4<f32>(
                min(color.r * 1.2, 1.0),
                min(color.g * 1.2, 1.0),
                min(color.b * 1.2, 1.0),
                color.a
            );
        }
    }

    // Subtle gradient effect (lighter at top)
    let gradient = 0.9 + 0.1 * in.bar_uv.y;
    color = vec4<f32>(color.rgb * gradient, color.a);

    // Apply alpha and anti-aliasing
    let final_alpha = color.a * aa;

    // Premultiply alpha for correct blending
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
