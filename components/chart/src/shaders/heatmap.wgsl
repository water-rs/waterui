// Heatmap chart shader.
//
// Renders a 2D grid of cells colored by value using a color scale.
// Uses instanced rendering where each instance is a cell.
//
// Supports:
// - Multiple color scales (Viridis, Plasma, Heat, custom)
// - Animation interpolation
// - Hover highlighting

// Uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Grid: [rows, cols, min_value, max_value]
    grid: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, 0] - normalized coordinates, -1 if not hovering
    pointer: vec4<f32>,
    // Zoom/Pan: [scale, offset_x, offset_y, 0]
    zoom_pan: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<storage, read> values: array<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) cell_uv: vec2<f32>,
    @location(2) @interpolate(flat) cell_index: u32,
}

// Viridis color scale (perceptually uniform)
fn viridis(t: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.267004, 0.004874, 0.329415);
    let c1 = vec3<f32>(0.282327, 0.140926, 0.457517);
    let c2 = vec3<f32>(0.253935, 0.265254, 0.529983);
    let c3 = vec3<f32>(0.206756, 0.371758, 0.553117);
    let c4 = vec3<f32>(0.143936, 0.462840, 0.543820);
    let c5 = vec3<f32>(0.119699, 0.564522, 0.498257);
    let c6 = vec3<f32>(0.208030, 0.652940, 0.417757);
    let c7 = vec3<f32>(0.421908, 0.733545, 0.296198);
    let c8 = vec3<f32>(0.699415, 0.792240, 0.191769);
    let c9 = vec3<f32>(0.993248, 0.906157, 0.143936);

    let tt = clamp(t, 0.0, 1.0) * 9.0;
    let i = u32(tt);
    let f = fract(tt);

    var colors: array<vec3<f32>, 10>;
    colors[0] = c0; colors[1] = c1; colors[2] = c2; colors[3] = c3; colors[4] = c4;
    colors[5] = c5; colors[6] = c6; colors[7] = c7; colors[8] = c8; colors[9] = c9;

    let i_clamped = min(i, 8u);
    return mix(colors[i_clamped], colors[i_clamped + 1u], f);
}

// Normalize value to [0, 1] range
fn normalize_value(value: f32) -> f32 {
    let min_val = uniforms.grid.z;
    let max_val = uniforms.grid.w;
    let range = max_val - min_val;
    return select(
        (value - min_val) / range,
        0.5,
        range <= 0.0
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let rows = u32(uniforms.grid.x);
    let cols = u32(uniforms.grid.y);
    let total_cells = rows * cols;

    if instance_index >= total_cells {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get cell position
    let row = instance_index / cols;
    let col = instance_index % cols;

    // Get value and apply animation
    let value = values[instance_index];
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    var animated_value = value;
    if uniforms.animation.w > 0.5 {
        // Entry animation: fade in from center value
        let center = (uniforms.grid.z + uniforms.grid.w) * 0.5;
        animated_value = mix(center, value, eased_progress);
    }

    // Normalize and get color
    let norm_value = normalize_value(animated_value);
    let color = viridis(norm_value);

    // Calculate cell dimensions
    let cell_width = 1.0 / f32(cols);
    let cell_height = 1.0 / f32(rows);

    // Cell position (bottom-left corner)
    let cell_x = f32(col) * cell_width;
    let cell_y = f32(row) * cell_height;

    // Generate quad vertices
    var local_x: f32;
    var local_y: f32;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { local_x = 0.0; local_y = 0.0; uv = vec2(0.0, 0.0); }
        case 1u: { local_x = 0.0; local_y = 1.0; uv = vec2(0.0, 1.0); }
        case 2u: { local_x = 1.0; local_y = 1.0; uv = vec2(1.0, 1.0); }
        case 3u: { local_x = 0.0; local_y = 0.0; uv = vec2(0.0, 0.0); }
        case 4u: { local_x = 1.0; local_y = 1.0; uv = vec2(1.0, 1.0); }
        case 5u: { local_x = 1.0; local_y = 0.0; uv = vec2(1.0, 0.0); }
        default: { local_x = 0.0; local_y = 0.0; uv = vec2(0.0); }
    }

    // Transform to chart space
    let chart_x = cell_x + local_x * cell_width;
    let chart_y = cell_y + local_y * cell_height;

    // Apply zoom/pan transformation
    let scale = uniforms.zoom_pan.x;
    let offset_x = uniforms.zoom_pan.y;
    let offset_y = uniforms.zoom_pan.z;
    let zoomed_x = (chart_x - 0.5) * scale + 0.5 + offset_x;
    let zoomed_y = (chart_y - 0.5) * scale + 0.5 + offset_y;

    // Apply padding and convert to NDC
    let padding = 0.05;
    let ndc_x = (zoomed_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (zoomed_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = vec4<f32>(color, 1.0);
    out.cell_uv = uv;
    out.cell_index = instance_index;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    let rows = u32(uniforms.grid.x);
    let cols = u32(uniforms.grid.y);

    // Add cell border (subtle grid lines) with AA
    let border_width = 0.02;
    let edge_dist = min(min(in.cell_uv.x, 1.0 - in.cell_uv.x), min(in.cell_uv.y, 1.0 - in.cell_uv.y));
    let border_alpha = sdf_coverage(edge_dist - border_width);
    if border_alpha > 0.001 {
        let border_color = vec4<f32>(color.rgb * 0.7, color.a);
        color = mix(color, border_color, border_alpha);
    }

    // Hover highlight
    if uniforms.pointer.x >= 0.0 {
        let padding = 0.05;
        let pointer_x = (uniforms.pointer.x - padding) / (1.0 - 2.0 * padding);
        let pointer_y = (uniforms.pointer.y - padding) / (1.0 - 2.0 * padding);

        let hover_col = u32(pointer_x * f32(cols));
        let hover_row = u32(pointer_y * f32(rows));
        let hover_index = hover_row * cols + hover_col;

        if hover_index == in.cell_index {
            // Brighten hovered cell
            color = vec4<f32>(
                min(color.r * 1.3, 1.0),
                min(color.g * 1.3, 1.0),
                min(color.b * 1.3, 1.0),
                color.a
            );
        }
    }

    // Premultiply alpha
    return vec4<f32>(color.rgb * color.a, color.a);
}
