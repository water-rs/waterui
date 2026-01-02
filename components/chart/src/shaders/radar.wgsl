// Radar/Spider chart shader.
//
// Renders multivariate data on radial axes:
// - Axis lines (spokes) from center
// - Concentric grid rings
// - Filled polygons for data series
// - Polygon outlines

struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Config: [axis_count, series_count, max_value, ring_count]
    config: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Style: [line_width, fill_opacity, 0, 0]
    style: vec4<f32>,
}

// Per-series data: values packed sequentially
// Layout: [series0_val0, series0_val1, ..., series1_val0, ...]
@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<storage, read> values: array<f32>;
@group(0) @binding(2) var<storage, read> colors: array<vec4<f32>>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) element_type: u32,
}

const PI: f32 = 3.14159265359;
const TWO_PI: f32 = 6.28318530718;

// Get angle for axis index (starting from top, going clockwise)
fn axis_angle(axis: u32, axis_count: u32) -> f32 {
    return -PI / 2.0 + TWO_PI * f32(axis) / f32(axis_count);
}

// Convert polar to cartesian
fn polar_to_cart(angle: f32, radius: f32) -> vec2<f32> {
    return vec2<f32>(cos(angle), sin(angle)) * radius;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let axis_count = u32(uniforms.config.x);
    let series_count = u32(uniforms.config.y);
    let max_value = uniforms.config.z;
    let ring_count = u32(uniforms.config.w);
    let line_width = uniforms.style.x;
    let fill_opacity = uniforms.style.y;

    // Layout of instances:
    // [0, ring_count): grid rings
    // [ring_count, ring_count + axis_count): axis lines
    // [ring_count + axis_count, ring_count + axis_count + series_count): series fills
    // [ring_count + axis_count + series_count, ...): series outlines

    let ring_end = ring_count;
    let axis_end = ring_end + axis_count;
    let fill_end = axis_end + series_count;
    let outline_end = fill_end + series_count;

    let center = vec2<f32>(0.0, 0.0);
    let chart_radius = 0.4; // Chart radius in NDC

    // Animation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));
    let anim_scale = select(1.0, eased_progress, uniforms.animation.w > 0.5);

    var pos = vec2<f32>(0.0);
    var color = vec4<f32>(0.5, 0.5, 0.5, 0.3);
    var element_type = 0u;

    if instance_index < ring_end {
        // Grid ring
        element_type = 0u;
        let ring_idx = instance_index;
        let ring_radius = chart_radius * f32(ring_idx + 1u) / f32(ring_count);

        // Each ring is rendered as triangles forming a thin annulus
        // 6 vertices per segment, axis_count segments
        let segment = vertex_index / 6u;
        let local_vert = vertex_index % 6u;

        if segment >= axis_count {
            out.position = vec4<f32>(0.0);
            return out;
        }

        let angle0 = axis_angle(segment, axis_count);
        let angle1 = axis_angle(segment + 1u, axis_count);

        let inner_radius = ring_radius - line_width * 0.3 / uniforms.viewport.x;
        let outer_radius = ring_radius + line_width * 0.3 / uniforms.viewport.x;

        switch local_vert {
            case 0u: { pos = polar_to_cart(angle0, inner_radius); }
            case 1u: { pos = polar_to_cart(angle0, outer_radius); }
            case 2u: { pos = polar_to_cart(angle1, outer_radius); }
            case 3u: { pos = polar_to_cart(angle0, inner_radius); }
            case 4u: { pos = polar_to_cart(angle1, outer_radius); }
            case 5u: { pos = polar_to_cart(angle1, inner_radius); }
            default: {}
        }

        color = vec4<f32>(0.5, 0.5, 0.5, 0.3);

    } else if instance_index < axis_end {
        // Axis line (spoke)
        element_type = 1u;
        let axis_idx = instance_index - ring_end;
        let angle = axis_angle(axis_idx, axis_count);

        // 6 vertices for the line quad
        let local_vert = vertex_index % 6u;
        let dir = polar_to_cart(angle, 1.0);
        let perp = vec2<f32>(-dir.y, dir.x) * line_width * 0.5 / uniforms.viewport.x;

        let p0 = center;
        let p1 = center + dir * chart_radius;

        switch local_vert {
            case 0u: { pos = p0 - perp; }
            case 1u: { pos = p0 + perp; }
            case 2u: { pos = p1 + perp; }
            case 3u: { pos = p0 - perp; }
            case 4u: { pos = p1 + perp; }
            case 5u: { pos = p1 - perp; }
            default: {}
        }

        color = vec4<f32>(0.6, 0.6, 0.6, 0.5);

    } else if instance_index < fill_end {
        // Series fill (polygon)
        element_type = 2u;
        let series_idx = instance_index - axis_end;

        // Triangle fan from center: 3 vertices per triangle, axis_count triangles
        let tri_idx = vertex_index / 3u;
        let local_vert = vertex_index % 3u;

        if tri_idx >= axis_count {
            out.position = vec4<f32>(0.0);
            return out;
        }

        let val_base = series_idx * axis_count;
        let val0 = values[val_base + tri_idx] / max_value;
        let val1 = values[val_base + ((tri_idx + 1u) % axis_count)] / max_value;

        let angle0 = axis_angle(tri_idx, axis_count);
        let angle1 = axis_angle(tri_idx + 1u, axis_count);

        let r0 = chart_radius * clamp(val0, 0.0, 1.0) * anim_scale;
        let r1 = chart_radius * clamp(val1, 0.0, 1.0) * anim_scale;

        switch local_vert {
            case 0u: { pos = center; }
            case 1u: { pos = polar_to_cart(angle0, r0); }
            case 2u: { pos = polar_to_cart(angle1, r1); }
            default: {}
        }

        color = colors[series_idx];
        color.a *= fill_opacity;

    } else if instance_index < outline_end {
        // Series outline
        element_type = 3u;
        let series_idx = instance_index - fill_end;

        // 6 vertices per edge, axis_count edges
        let edge_idx = vertex_index / 6u;
        let local_vert = vertex_index % 6u;

        if edge_idx >= axis_count {
            out.position = vec4<f32>(0.0);
            return out;
        }

        let val_base = series_idx * axis_count;
        let val0 = values[val_base + edge_idx] / max_value;
        let val1 = values[val_base + ((edge_idx + 1u) % axis_count)] / max_value;

        let angle0 = axis_angle(edge_idx, axis_count);
        let angle1 = axis_angle(edge_idx + 1u, axis_count);

        let r0 = chart_radius * clamp(val0, 0.0, 1.0) * anim_scale;
        let r1 = chart_radius * clamp(val1, 0.0, 1.0) * anim_scale;

        let p0 = polar_to_cart(angle0, r0);
        let p1 = polar_to_cart(angle1, r1);

        let edge_dir = normalize(p1 - p0);
        let edge_perp = vec2<f32>(-edge_dir.y, edge_dir.x) * line_width / uniforms.viewport.x;

        switch local_vert {
            case 0u: { pos = p0 - edge_perp; }
            case 1u: { pos = p0 + edge_perp; }
            case 2u: { pos = p1 + edge_perp; }
            case 3u: { pos = p0 - edge_perp; }
            case 4u: { pos = p1 + edge_perp; }
            case 5u: { pos = p1 - edge_perp; }
            default: {}
        }

        color = colors[series_idx];
        color.a = 1.0; // Full opacity for outline

    } else {
        out.position = vec4<f32>(0.0);
        return out;
    }

    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.color = color;
    out.uv = pos;
    out.element_type = element_type;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // Premultiply alpha
    return vec4<f32>(color.rgb * color.a, color.a);
}
