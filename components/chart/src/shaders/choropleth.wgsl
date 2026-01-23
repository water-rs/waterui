// Choropleth map shader for geographic visualization.
// Renders tessellated polygons colored by data values.

struct Uniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Geographic bounds: [min_lon, max_lon, min_lat, max_lat]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, hover_id]
    pointer: vec4<f32>,
    // Value range: [min_value, max_value, stroke_width, show_stroke]
    value_range: vec4<f32>,
    // Stroke color
    stroke_color: vec4<f32>,
}

struct Vertex {
    // Position: [x, y]
    pos: vec2<f32>,
    // Value and polygon ID
    value: f32,
    polygon_id: f32,
}

struct ColorStop {
    position: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(2) var<storage, read> prev_vertices: array<Vertex>;
@group(0) @binding(3) var<storage, read> color_stops: array<ColorStop>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) polygon_id: f32,
    @location(2) world_pos: vec2<f32>,
}

// Sample color from color scale
fn sample_color_scale(t: f32, num_stops: u32) -> vec4<f32> {
    if num_stops == 0u {
        return vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }

    let clamped_t = clamp(t, 0.0, 1.0);

    // Find the two stops to interpolate between
    var prev_stop = color_stops[0];
    var next_stop = color_stops[0];

    for (var i = 0u; i < num_stops; i++) {
        let stop = color_stops[i];
        if stop.position <= clamped_t {
            prev_stop = stop;
        }
        if stop.position >= clamped_t {
            next_stop = stop;
            break;
        }
        next_stop = stop;
    }

    // Interpolate between stops
    let range = next_stop.position - prev_stop.position;
    if range <= 0.0 {
        return prev_stop.color;
    }

    let local_t = (clamped_t - prev_stop.position) / range;
    return mix(prev_stop.color, next_stop.color, local_t);
}

// Apply easing function
fn ease(t: f32, easing: f32) -> f32 {
    let easing_type = i32(easing);
    switch easing_type {
        case 0: { // Linear
            return t;
        }
        case 1: { // Ease out cubic
            let inv = 1.0 - t;
            return 1.0 - inv * inv * inv;
        }
        case 2: { // Ease in out cubic
            if t < 0.5 {
                return 4.0 * t * t * t;
            } else {
                let f = 2.0 * t - 2.0;
                return 0.5 * f * f * f + 1.0;
            }
        }
        case 3: { // Ease out elastic
            if t <= 0.0 || t >= 1.0 {
                return t;
            }
            return pow(2.0, -10.0 * t) * sin((t * 10.0 - 0.75) * 2.0944) + 1.0;
        }
        default: {
            return t;
        }
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let curr = vertices[vertex_index];
    let prev = prev_vertices[vertex_index];

    // Animation interpolation
    let progress = ease(uniforms.animation.y, uniforms.animation.z);
    let entry_active = uniforms.animation.w;

    // Interpolate position
    var pos = mix(prev.pos, curr.pos, progress);

    // Entry animation: scale from center
    if entry_active > 0.5 {
        let bounds = uniforms.bounds;
        let center = vec2<f32>(
            (bounds.x + bounds.y) * 0.5,
            (bounds.z + bounds.w) * 0.5
        );
        pos = mix(center, pos, progress);
    }

    // Interpolate value
    let value = mix(prev.value, curr.value, progress);

    // Transform to normalized device coordinates
    let bounds = uniforms.bounds;
    let lon_range = bounds.y - bounds.x;
    let lat_range = bounds.w - bounds.z;

    // Apply Mercator-like projection (simple linear for now)
    var ndc = vec2<f32>(
        (pos.x - bounds.x) / lon_range * 2.0 - 1.0,
        (pos.y - bounds.z) / lat_range * 2.0 - 1.0
    );

    // Flip Y for screen coordinates
    ndc.y = -ndc.y;

    // Normalize value for color mapping
    let value_range = uniforms.value_range;
    let normalized_value = (value - value_range.x) / (value_range.y - value_range.x);

    // Get color from color scale
    let stop_count = arrayLength(&color_stops);
    let color = sample_color_scale(normalized_value, stop_count);

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.color = color;
    output.polygon_id = curr.polygon_id;
    output.world_pos = pos;

    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = input.color;

    // Hover highlight
    let hover_id = uniforms.pointer.w;
    if hover_id >= 0.0 && abs(input.polygon_id - hover_id) < 0.5 {
        // Brighten on hover
        color = vec4<f32>(
            min(color.r * 1.2 + 0.1, 1.0),
            min(color.g * 1.2 + 0.1, 1.0),
            min(color.b * 1.2 + 0.1, 1.0),
            color.a
        );
    }

    return color;
}

// Separate pass for polygon outlines/strokes
struct StrokeVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) edge_dist: f32,
}

@vertex
fn vs_stroke(@builtin(vertex_index) vertex_index: u32) -> StrokeVertexOutput {
    // This would be called with line geometry for borders
    // For now, we'll handle strokes in the fragment shader using SDF
    var output: StrokeVertexOutput;
    output.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    output.edge_dist = 0.0;
    return output;
}

@fragment
fn fs_stroke(input: StrokeVertexOutput) -> @location(0) vec4<f32> {
    let stroke_color = uniforms.stroke_color;
    let stroke_width = uniforms.value_range.z;

    // Anti-aliased stroke
    let edge_dist = abs(input.edge_dist) - stroke_width * 0.5;
    let alpha = sdf_coverage(edge_dist);

    return vec4<f32>(stroke_color.rgb, stroke_color.a * alpha);
}
