// Gauge chart shader.
//
// Renders a circular speedometer-style gauge with arc segments.
// Supports animated value changes and multiple arc regions.

struct GaugeUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Config: [start_angle, end_angle, inner_radius, outer_radius]
    config: vec4<f32>,
    // Value: [current_value, previous_value, min_value, max_value]
    value: vec4<f32>,
    // Needle: [needle_width, needle_length, show_needle, 0]
    needle: vec4<f32>,
    // Background color
    background_color: vec4<f32>,
    // Value arc color
    value_color: vec4<f32>,
    // Needle color
    needle_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: GaugeUniforms;
@group(0) @binding(1) var<storage, read> region_colors: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> region_thresholds: array<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Note: Easing functions provided by common.wgsl (prepended at compile time)

const PI: f32 = 3.14159265359;
const TWO_PI: f32 = 6.28318530718;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    // Full-screen quad
    var positions = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
        vec2(-1.0, 1.0)
    );

    var uvs = array<vec2<f32>, 6>(
        vec2(0.0, 1.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(1.0, 1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 0.0)
    );

    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];

    return out;
}

// Calculate angle from UV coordinates (center is 0.5, 0.5)
fn uv_to_angle(uv: vec2<f32>) -> f32 {
    let centered = uv - vec2(0.5);
    return atan2(centered.x, -centered.y);
}

// Check if angle is within gauge range
fn angle_in_range(angle: f32, start: f32, end: f32) -> bool {
    // Normalize angles to handle wraparound
    var a = angle;
    var s = start;
    var e = end;

    // Handle the case where the arc crosses the -PI/PI boundary
    if s > e {
        if a < 0.0 {
            a += TWO_PI;
        }
        if e < 0.0 {
            e += TWO_PI;
        }
        if s < 0.0 {
            s += TWO_PI;
        }
    }

    return a >= s && a <= e;
}

// Map value to angle
fn value_to_angle(value: f32) -> f32 {
    let normalized = (value - uniforms.value.z) / (uniforms.value.w - uniforms.value.z);
    let clamped = clamp(normalized, 0.0, 1.0);
    return uniforms.config.x + clamped * (uniforms.config.y - uniforms.config.x);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let centered = in.uv - vec2(0.5);
    let dist = length(centered);
    let angle = atan2(centered.x, -centered.y);

    // Get radii from config
    let inner_radius = uniforms.config.z;
    let outer_radius = uniforms.config.w;
    let start_angle = uniforms.config.x;
    let end_angle = uniforms.config.y;

    // Apply animation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    // Interpolate value
    let current_value = uniforms.value.x;
    let previous_value = uniforms.value.y;
    let animated_value = mix(previous_value, current_value, eased_progress);
    let value_angle = value_to_angle(animated_value);

    // Check if in arc region
    let in_arc = dist >= inner_radius && dist <= outer_radius;

    // Normalize angle for range check
    var check_angle = angle;
    if check_angle < start_angle && start_angle > end_angle {
        check_angle += TWO_PI;
    }

    var color = vec4<f32>(0.0);

    if in_arc {
        // Check if within gauge angle range
        let in_gauge_range = angle_in_range(angle, start_angle, end_angle);

        if in_gauge_range {
            // Check if before or after value angle
            let in_value_range = angle_in_range(angle, start_angle, value_angle);

            if in_value_range {
                // Value arc - check for colored regions
                let normalized_value = (animated_value - uniforms.value.z) / (uniforms.value.w - uniforms.value.z);
                let region_count = arrayLength(&region_colors);

                if region_count > 0u {
                    // Find which region this value falls into
                    var region_color = uniforms.value_color;
                    for (var i = 0u; i < region_count; i++) {
                        let threshold = region_thresholds[i];
                        if normalized_value <= threshold {
                            region_color = region_colors[i];
                            break;
                        }
                        if i == region_count - 1u {
                            region_color = region_colors[i];
                        }
                    }
                    color = region_color;
                } else {
                    color = uniforms.value_color;
                }
            } else {
                // Background arc
                color = uniforms.background_color;
            }

            // Anti-aliasing at edges
            let edge_softness = 0.005;
            let inner_aa = smoothstep(inner_radius - edge_softness, inner_radius + edge_softness, dist);
            let outer_aa = smoothstep(outer_radius + edge_softness, outer_radius - edge_softness, dist);
            color.a *= inner_aa * outer_aa;
        }
    }

    // Draw needle if enabled
    if uniforms.needle.z > 0.5 {
        let needle_width = uniforms.needle.x;
        let needle_length = uniforms.needle.y;

        // Needle angle points to current value
        let needle_angle = value_angle;

        // Calculate distance from the needle line
        let needle_dir = vec2(sin(needle_angle), -cos(needle_angle));
        let to_point = centered;
        let along_needle = dot(to_point, needle_dir);
        let perp_dist = abs(dot(to_point, vec2(-needle_dir.y, needle_dir.x)));

        // Check if within needle bounds
        let center_radius = inner_radius * 0.3;
        if along_needle > center_radius && along_needle < needle_length && perp_dist < needle_width * 0.5 {
            // Needle body
            let needle_aa = smoothstep(needle_width * 0.5, needle_width * 0.5 - 0.003, perp_dist);
            let tip_fade = smoothstep(needle_length, needle_length - 0.02, along_needle);
            let needle_alpha = needle_aa * tip_fade;

            color = mix(color, uniforms.needle_color, needle_alpha);
        }

        // Center cap
        if dist < center_radius {
            let cap_aa = smoothstep(center_radius, center_radius - 0.005, dist);
            color = mix(color, uniforms.needle_color, cap_aa);
        }
    }

    // Premultiply alpha
    return vec4<f32>(color.rgb * color.a, color.a);
}
