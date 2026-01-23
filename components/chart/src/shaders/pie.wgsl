// Pie/donut chart shader.
//
// Renders pie slices using a fullscreen quad approach. The fragment shader
// calculates which slice each pixel belongs to based on angle and distance.
//
// Supports:
// - Donut charts (configurable inner radius)
// - Hover highlighting with slice "pop out" effect
// - Anti-aliased edges

// Pie uniforms
struct PieUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, hovered_slice]
    pointer: vec4<f32>,
    // Config: [inner_radius, start_angle, slice_count, 0]
    config: vec4<f32>,
}

// Per-slice data - matches GpuSliceData struct
struct SliceData {
    // Angles: [start_angle, end_angle]
    angles: vec2<f32>,
    // Value (for animation)
    value: f32,
    // Padding
    _pad: f32,
    // Color: [r, g, b, a]
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: PieUniforms;
@group(0) @binding(1) var<storage, read> slices: array<SliceData>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Normalize angle to [0, TAU)
fn normalize_angle(a: f32) -> f32 {
    var angle = a;
    while angle < 0.0 {
        angle += TAU;
    }
    while angle >= TAU {
        angle -= TAU;
    }
    return angle;
}

// Check if angle is within a slice (handling wraparound)
fn angle_in_slice(angle: f32, start: f32, end: f32) -> bool {
    let norm_angle = normalize_angle(angle);
    let norm_start = normalize_angle(start);
    let norm_end = normalize_angle(end);

    if norm_start <= norm_end {
        return norm_angle >= norm_start && norm_angle < norm_end;
    } else {
        // Slice wraps around 0
        return norm_angle >= norm_start || norm_angle < norm_end;
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    // Fullscreen quad vertices
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
    );

    let pos = positions[vertex_index];
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + 0.5; // Map to [0, 1]

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let slice_count = u32(uniforms.config.z);
    if slice_count == 0u {
        return vec4<f32>(0.0);
    }

    // Convert UV to centered coordinates with aspect ratio correction
    let aspect = uniforms.viewport.x / uniforms.viewport.y;
    var centered = (in.uv - 0.5) * 2.0;
    if aspect > 1.0 {
        centered.x *= aspect;
    } else {
        centered.y /= aspect;
    }

    // Calculate distance and angle from center
    let dist = length(centered);
    let angle = atan2(centered.y, centered.x);

    // Pie radius (90% of half the smaller dimension)
    let outer_radius = 0.9;
    let inner_radius = uniforms.config.x * outer_radius;

    // Anti-aliasing width in normalized units
    let edge_width = min(uniforms.viewport.z, uniforms.viewport.w) * 1.5;

    // Hovered slice index
    let hovered_slice = i32(uniforms.pointer.w);

    // Find which slice this pixel belongs to
    var result_color = vec4<f32>(0.0);

    for (var i = 0u; i < slice_count; i++) {
        let slice = slices[i];
        let start_angle = slice.angles.x;
        let end_angle = slice.angles.y;

        // Check if this angle is in the slice
        if !angle_in_slice(angle, start_angle, end_angle) {
            continue;
        }

        // Calculate effective radii for this slice
        var slice_outer = outer_radius;
        var slice_inner = inner_radius;

        // Hover effect: "pop out" the hovered slice
        if i32(i) == hovered_slice {
            let pop_distance = 0.05;
            let slice_mid_angle = (start_angle + end_angle) * 0.5;
            let pop_offset = vec2<f32>(cos(slice_mid_angle), sin(slice_mid_angle)) * pop_distance;

            // Adjust centered coordinates for pop effect
            let adjusted_centered = centered - pop_offset;
            let adjusted_dist = length(adjusted_centered);
            let adjusted_angle = atan2(adjusted_centered.y, adjusted_centered.x);

            // SDF for hovered slice
            let radial_dist = sdf_ring(adjusted_centered, slice_inner, slice_outer);
            let angle_from_start = normalize_angle(adjusted_angle - start_angle);
            let angle_to_end = normalize_angle(end_angle - adjusted_angle);
            let angular_dist = min(angle_from_start, angle_to_end) * adjusted_dist;
            let combined_dist = max(radial_dist, -angular_dist);

            // Anti-aliased coverage
            let aa = sdf_fill(combined_dist, edge_width);

            if aa > 0.001 {
                // Brighten hovered slice
                var color = slice.color;
                color = vec4<f32>(
                    min(color.r * 1.15, 1.0),
                    min(color.g * 1.15, 1.0),
                    min(color.b * 1.15, 1.0),
                    color.a
                );

                result_color = vec4<f32>(color.rgb * aa, aa);
            }
            continue;
        }

        // Regular slice rendering - use SDF for anti-aliasing
        // Radial SDF: distance from the ring (inner/outer radius)
        let radial_dist = sdf_ring(centered, slice_inner, slice_outer);

        // Angular SDF: distance from slice edges
        let angle_from_start = normalize_angle(angle - start_angle);
        let angle_to_end = normalize_angle(end_angle - angle);
        let angular_dist = min(angle_from_start, angle_to_end) * dist;  // Convert to linear distance

        // Combined SDF
        let combined_dist = max(radial_dist, -angular_dist);

        // Anti-aliased coverage
        let aa = sdf_fill(combined_dist, edge_width);

        if aa > 0.001 {
            let color = slice.color;
            result_color = vec4<f32>(color.rgb * aa, aa);
        }
    }

    return result_color;
}
