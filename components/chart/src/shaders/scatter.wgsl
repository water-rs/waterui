// Scatter chart shader.
//
// Renders points as circles using instanced rendering. Each instance is a
// point, drawn as a quad with a circular alpha mask.
//
// Supports:
// - Animation interpolation between previous and current data
// - Entry animation (points grow from center)
// - Hover highlighting

// Chart uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, point_radius]
    pointer: vec4<f32>,
}

// Data point
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
    @location(1) uv: vec2<f32>,           // UV for circle mask
    @location(2) point_index: f32,        // Which point this is
}

// Note: Easing functions provided by common.wgsl (prepended at compile time)

// Normalize Y value to [0, 1] based on data bounds
fn normalize_y(y: f32) -> f32 {
    let y_range = uniforms.bounds.w - uniforms.bounds.z;
    return select(
        (y - uniforms.bounds.z) / y_range,
        0.0,
        y_range <= 0.0
    );
}

// Normalize X value to [0, 1] based on data bounds
fn normalize_x(x: f32) -> f32 {
    let x_range = uniforms.bounds.y - uniforms.bounds.x;
    return select(
        (x - uniforms.bounds.x) / x_range,
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
    if instance_index >= point_count {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get current and previous points
    let curr = current_data[instance_index];
    let prev = previous_data[instance_index];

    // Apply animation interpolation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    let x = mix(prev.x, curr.x, eased_progress);
    let y = mix(prev.y, curr.y, eased_progress);

    // Normalize to chart coordinates
    let norm_x = normalize_x(x);
    let norm_y = normalize_y(y);

    // Point radius in pixels, then convert to normalized coordinates
    let point_radius = uniforms.pointer.w;
    let radius_x = point_radius / uniforms.viewport.x;
    let radius_y = point_radius / uniforms.viewport.y;

    // Entry animation: points grow from nothing
    var scale = 1.0;
    if uniforms.animation.w > 0.5 {
        scale = eased_progress;
    }

    // Generate quad vertices around the point center
    // Vertices 0-2: first triangle, 3-5: second triangle
    var local_offset: vec2<f32>;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { local_offset = vec2(-1.0, -1.0); uv = vec2(0.0, 0.0); }
        case 1u: { local_offset = vec2( 1.0, -1.0); uv = vec2(1.0, 0.0); }
        case 2u: { local_offset = vec2(-1.0,  1.0); uv = vec2(0.0, 1.0); }
        case 3u: { local_offset = vec2(-1.0,  1.0); uv = vec2(0.0, 1.0); }
        case 4u: { local_offset = vec2( 1.0, -1.0); uv = vec2(1.0, 0.0); }
        case 5u: { local_offset = vec2( 1.0,  1.0); uv = vec2(1.0, 1.0); }
        default: { local_offset = vec2(0.0); uv = vec2(0.0); }
    }

    // Apply scale and offset
    local_offset *= scale;

    // Map to NDC with padding
    let padding = 0.1;
    let center_x = (norm_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let center_y = (norm_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    let ndc_x = center_x + local_offset.x * radius_x * 2.0;
    let ndc_y = center_y + local_offset.y * radius_y * 2.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = curr.color;
    out.uv = uv;
    out.point_index = f32(instance_index);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // SDF circle: signed distance from edge (negative inside, positive outside)
    let centered = (in.uv - 0.5) * 2.0;
    let dist = sdf_circle(centered, 1.0);

    // Anti-aliased coverage using fwidth for resolution-independent AA
    let aa = sdf_coverage(dist);

    // Early discard for fully transparent pixels
    if aa < 0.001 {
        discard;
    }

    // Check if pointer is near this point for hover effect
    if uniforms.pointer.x >= 0.0 {
        let point_count = f32(arrayLength(&current_data));
        let point_norm_x = f32(u32(in.point_index)) / max(point_count - 1.0, 1.0);

        // Simple hover check based on x proximity
        let hover_dist = abs(uniforms.pointer.x - point_norm_x);
        if hover_dist < 0.05 {
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
