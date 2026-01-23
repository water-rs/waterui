// Contour chart shader.
//
// Implements the marching squares algorithm on GPU for isoline visualization.
// Each instance processes one grid cell for one contour level.
//
// The algorithm:
// 1. Sample 4 corners of each cell
// 2. Classify corners as above/below threshold (4-bit case index)
// 3. Look up edge crossings from marching squares table
// 4. Interpolate crossing positions along edges
// 5. Draw line segments as anti-aliased quads

struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Grid: [rows, cols, min_value, max_value]
    grid: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Style: [line_width, num_levels, 0, 0]
    style: vec4<f32>,
    // Zoom/Pan: [scale, offset_x, offset_y, 0]
    zoom_pan: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<storage, read> values: array<f32>;
@group(0) @binding(2) var<storage, read> levels: array<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) line_uv: vec2<f32>,
    @location(2) @interpolate(flat) level_idx: u32,
}

// Viridis color scale for level coloring
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

// Marching squares edge table
// For each of 16 cases, encodes which edges have line crossings
// Edges: 0=bottom, 1=right, 2=top, 3=left
// Each case has 0, 1, or 2 line segments (up to 4 edge crossings)
// Format: 4 bits for edge count, then edge pairs
fn get_edge_table(case_idx: u32) -> vec4<u32> {
    // Edge indices: 0=bottom (v0-v1), 1=right (v1-v2), 2=top (v2-v3), 3=left (v3-v0)
    // Returns: (num_segments, edge1_start, edge1_end, edge2_start<<16|edge2_end)
    switch case_idx {
        // No edges (all corners same side)
        case 0u, 15u: { return vec4<u32>(0u, 0u, 0u, 0u); }
        // Single edge cases (1 corner different)
        case 1u: { return vec4<u32>(1u, 0u, 3u, 0u); }  // bottom-left
        case 2u: { return vec4<u32>(1u, 0u, 1u, 0u); }  // bottom-right
        case 4u: { return vec4<u32>(1u, 1u, 2u, 0u); }  // top-right
        case 8u: { return vec4<u32>(1u, 2u, 3u, 0u); }  // top-left
        // Two adjacent corners (straight line across)
        case 3u: { return vec4<u32>(1u, 1u, 3u, 0u); }  // left-right
        case 6u: { return vec4<u32>(1u, 0u, 2u, 0u); }  // bottom-top
        case 9u: { return vec4<u32>(1u, 0u, 2u, 0u); }  // bottom-top
        case 12u: { return vec4<u32>(1u, 1u, 3u, 0u); } // left-right
        // Two opposite corners (two segments) - saddle points
        case 5u: { return vec4<u32>(2u, 0u, 1u, (2u << 16u) | 3u); }
        case 10u: { return vec4<u32>(2u, 0u, 3u, (1u << 16u) | 2u); }
        // Three corners (complement of single corner)
        case 7u: { return vec4<u32>(1u, 2u, 3u, 0u); }
        case 11u: { return vec4<u32>(1u, 1u, 2u, 0u); }
        case 13u: { return vec4<u32>(1u, 0u, 1u, 0u); }
        case 14u: { return vec4<u32>(1u, 0u, 3u, 0u); }
        default: { return vec4<u32>(0u, 0u, 0u, 0u); }
    }
}

// Interpolate position along an edge
fn interpolate_edge(edge: u32, v0: f32, v1: f32, v2: f32, v3: f32, threshold: f32, cell_x: f32, cell_y: f32, cell_w: f32, cell_h: f32) -> vec2<f32> {
    switch edge {
        case 0u: { // Bottom edge (v0 to v1)
            let denom = v1 - v0;
            let t = select(0.5, (threshold - v0) / denom, abs(denom) > 1e-6);
            return vec2<f32>(cell_x + t * cell_w, cell_y);
        }
        case 1u: { // Right edge (v1 to v2)
            let denom = v2 - v1;
            let t = select(0.5, (threshold - v1) / denom, abs(denom) > 1e-6);
            return vec2<f32>(cell_x + cell_w, cell_y + t * cell_h);
        }
        case 2u: { // Top edge (v2 to v3)
            let denom = v2 - v3;
            let t = select(0.5, (threshold - v3) / denom, abs(denom) > 1e-6);
            return vec2<f32>(cell_x + t * cell_w, cell_y + cell_h);
        }
        case 3u: { // Left edge (v3 to v0)
            let denom = v3 - v0;
            let t = select(0.5, (threshold - v0) / denom, abs(denom) > 1e-6);
            return vec2<f32>(cell_x, cell_y + t * cell_h);
        }
        default: { return vec2<f32>(0.0); }
    }
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let rows = u32(uniforms.grid.x);
    let cols = u32(uniforms.grid.y);
    let num_levels = u32(uniforms.style.y);
    let line_width = uniforms.style.x;

    // Decode instance: instance_index = level_idx * (rows-1) * (cols-1) + cell_idx
    let cells_per_level = (rows - 1u) * (cols - 1u);
    let level_idx = instance_index / cells_per_level;
    let cell_idx = instance_index % cells_per_level;

    if level_idx >= num_levels || cells_per_level == 0u {
        out.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        out.color = vec4<f32>(0.0);
        out.line_uv = vec2<f32>(0.0);
        out.level_idx = 0u;
        return out;
    }

    // Get cell position (row, col of bottom-left corner)
    let cell_row = cell_idx / (cols - 1u);
    let cell_col = cell_idx % (cols - 1u);

    // Get threshold for this level
    let threshold = levels[level_idx];

    // Sample 4 corners (counter-clockwise from bottom-left)
    let idx00 = cell_row * cols + cell_col;           // v0: bottom-left
    let idx10 = cell_row * cols + cell_col + 1u;      // v1: bottom-right
    let idx11 = (cell_row + 1u) * cols + cell_col + 1u; // v2: top-right
    let idx01 = (cell_row + 1u) * cols + cell_col;    // v3: top-left

    let v0 = values[idx00];
    let v1 = values[idx10];
    let v2 = values[idx11];
    let v3 = values[idx01];

    // Classify corners (1 if >= threshold, 0 otherwise)
    var case_idx = 0u;
    if v0 >= threshold { case_idx |= 1u; }
    if v1 >= threshold { case_idx |= 2u; }
    if v2 >= threshold { case_idx |= 4u; }
    if v3 >= threshold { case_idx |= 8u; }

    // Get edge table for this case
    let edge_info = get_edge_table(case_idx);
    let num_segments = edge_info.x;

    // Determine which segment this vertex belongs to
    // Each segment needs 6 vertices (2 triangles for line quad)
    let segment_idx = vertex_index / 6u;
    let local_vertex = vertex_index % 6u;

    if segment_idx >= num_segments {
        out.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        out.color = vec4<f32>(0.0);
        out.line_uv = vec2<f32>(0.0);
        out.level_idx = level_idx;
        return out;
    }

    // Calculate cell dimensions in normalized chart space
    let cell_w = 1.0 / f32(cols - 1u);
    let cell_h = 1.0 / f32(rows - 1u);
    let cell_x = f32(cell_col) * cell_w;
    let cell_y = f32(cell_row) * cell_h;

    // Get edges for this segment
    var edge_start: u32;
    var edge_end: u32;
    if segment_idx == 0u {
        edge_start = edge_info.y;
        edge_end = edge_info.z;
    } else {
        edge_start = (edge_info.w >> 16u) & 0xFFFFu;
        edge_end = edge_info.w & 0xFFFFu;
    }

    // Interpolate line endpoints
    var p0 = interpolate_edge(edge_start, v0, v1, v2, v3, threshold, cell_x, cell_y, cell_w, cell_h);
    var p1 = interpolate_edge(edge_end, v0, v1, v2, v3, threshold, cell_x, cell_y, cell_w, cell_h);

    // Calculate line direction and perpendicular (skip zero-length segments)
    let line_delta = p1 - p0;
    if length(line_delta) < 1e-6 {
        out.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        out.color = vec4<f32>(0.0);
        out.line_uv = vec2<f32>(0.0);
        out.level_idx = level_idx;
        return out;
    }
    let line_dir = normalize(line_delta);
    // Slightly extend segments to avoid visible gaps between adjacent cells
    let half_width = line_width * 0.5 / max(uniforms.viewport.x, uniforms.viewport.y);
    let overlap = max(half_width * 2.0, min(cell_w, cell_h) * 0.05);
    p0 = p0 - line_dir * overlap;
    p1 = p1 + line_dir * overlap;
    let line_perp = vec2<f32>(-line_dir.y, line_dir.x);

    // Generate quad vertices for anti-aliased line
    var offset: vec2<f32>;
    var uv: vec2<f32>;
    switch local_vertex {
        case 0u: { offset = p0 - line_perp * half_width; uv = vec2<f32>(0.0, 0.0); }
        case 1u: { offset = p0 + line_perp * half_width; uv = vec2<f32>(0.0, 1.0); }
        case 2u: { offset = p1 + line_perp * half_width; uv = vec2<f32>(1.0, 1.0); }
        case 3u: { offset = p0 - line_perp * half_width; uv = vec2<f32>(0.0, 0.0); }
        case 4u: { offset = p1 + line_perp * half_width; uv = vec2<f32>(1.0, 1.0); }
        case 5u: { offset = p1 - line_perp * half_width; uv = vec2<f32>(1.0, 0.0); }
        default: { offset = vec2<f32>(0.0); uv = vec2<f32>(0.0); }
    }

    // Apply animation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    // Entry animation: grow from center
    if uniforms.animation.w > 0.5 {
        let center = vec2<f32>(0.5, 0.5);
        offset = mix(center, offset, eased_progress);
    }

    // Apply zoom/pan transformation
    let scale = uniforms.zoom_pan.x;
    let pan_offset_x = uniforms.zoom_pan.y;
    let pan_offset_y = uniforms.zoom_pan.z;
    let zoomed_x = (offset.x - 0.5) * scale + 0.5 + pan_offset_x;
    let zoomed_y = (offset.y - 0.5) * scale + 0.5 + pan_offset_y;

    // Apply padding and convert to NDC
    let padding = 0.05;
    let ndc_x = (zoomed_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (zoomed_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    // Color based on level
    let level_ratio = f32(level_idx) / max(f32(num_levels - 1u), 1.0);
    let color = viridis(level_ratio);

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = vec4<f32>(color, 1.0);
    out.line_uv = uv;
    out.level_idx = level_idx;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Anti-aliased line rendering
    // uv.y goes from 0 (edge) to 0.5 (center) to 1 (edge)
    let dist_from_center = abs(in.line_uv.y - 0.5) * 2.0;
    let edge_dist = dist_from_center - 1.0;
    let alpha = sdf_coverage(edge_dist);

    let color = in.color;
    let final_color = vec4<f32>(color.rgb * alpha, alpha);

    // Premultiply alpha
    return vec4<f32>(final_color.rgb * final_color.a, final_color.a);
}
