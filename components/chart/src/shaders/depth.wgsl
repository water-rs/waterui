// Depth (order book) chart shader.
//
// Renders order book depth as two filled area charts:
// - Bids (buy orders) on the left, filled green
// - Asks (sell orders) on the right, filled red
//
// Uses a step-based rendering where each level creates a horizontal line
// at its cumulative volume, connected vertically to the next level.

// Uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_price, max_price, min_volume, max_volume]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, 0] - normalized coordinates, -1 if not hovering
    pointer: vec4<f32>,
}

// Depth level data
struct DepthLevel {
    price: f32,
    cumulative_volume: f32,
}

// Colors for bids/asks
struct DepthColors {
    bid_fill: vec4<f32>,
    bid_line: vec4<f32>,
    ask_fill: vec4<f32>,
    ask_line: vec4<f32>,
    mid_price: f32,
    line_width: f32,
    bid_count: f32,
    ask_count: f32,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<uniform> colors: DepthColors;
@group(0) @binding(2) var<storage, read> bids: array<DepthLevel>;
@group(0) @binding(3) var<storage, read> asks: array<DepthLevel>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) side: u32,  // 0 = bid, 1 = ask
}

// Normalize price to [0, 1] range
fn normalize_price(price: f32) -> f32 {
    let range = uniforms.bounds.y - uniforms.bounds.x;
    return select(
        (price - uniforms.bounds.x) / range,
        0.5,
        range <= 0.0
    );
}

// Normalize volume to [0, 1] range
fn normalize_volume(volume: f32) -> f32 {
    let range = uniforms.bounds.w - uniforms.bounds.z;
    return select(
        (volume - uniforms.bounds.z) / range,
        0.0,
        range <= 0.0
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let bid_count = u32(colors.bid_count);
    let ask_count = u32(colors.ask_count);
    let total_levels = bid_count + ask_count;

    if total_levels == 0u {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Animation progress
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));

    // Each level needs 6 vertices (2 triangles) to form a step
    // Plus we need triangles to fill down to the baseline
    let level_index = instance_index / 2u;
    let is_fill = (instance_index % 2u) == 0u;  // Alternating fill/step

    var is_bid: bool;
    var data_index: u32;

    if level_index < bid_count {
        is_bid = true;
        data_index = level_index;
    } else {
        is_bid = false;
        data_index = level_index - bid_count;
    }

    // Get level data
    var price: f32;
    var volume: f32;
    var prev_volume: f32 = 0.0;
    var next_price: f32;

    if is_bid {
        if data_index >= bid_count {
            out.position = vec4<f32>(0.0);
            return out;
        }
        let level = bids[data_index];
        price = level.price;
        volume = level.cumulative_volume;

        if data_index > 0u {
            prev_volume = bids[data_index - 1u].cumulative_volume;
        }
        if data_index + 1u < bid_count {
            next_price = bids[data_index + 1u].price;
        } else {
            next_price = uniforms.bounds.x;  // Extend to min price
        }
    } else {
        if data_index >= ask_count {
            out.position = vec4<f32>(0.0);
            return out;
        }
        let level = asks[data_index];
        price = level.price;
        volume = level.cumulative_volume;

        if data_index > 0u {
            prev_volume = asks[data_index - 1u].cumulative_volume;
        }
        if data_index + 1u < ask_count {
            next_price = asks[data_index + 1u].price;
        } else {
            next_price = uniforms.bounds.y;  // Extend to max price
        }
    }

    // Entry animation
    if uniforms.animation.w > 0.5 {
        volume = volume * eased_progress;
        prev_volume = prev_volume * eased_progress;
    }

    // Normalize coordinates
    let norm_price = normalize_price(price);
    let norm_next_price = normalize_price(next_price);
    let norm_volume = normalize_volume(volume);
    let norm_prev_volume = normalize_volume(prev_volume);

    // Generate step vertices
    // For each level, we create a step shape:
    // - Horizontal line at current volume from previous price to current price
    // - Vertical line from previous volume to current volume at current price
    var local_x: f32;
    var local_y: f32;

    if is_fill {
        // Fill triangle (from baseline to step)
        switch vertex_index {
            case 0u: { local_x = norm_price; local_y = 0.0; }
            case 1u: { local_x = norm_price; local_y = norm_volume; }
            case 2u: { local_x = norm_next_price; local_y = norm_volume; }
            case 3u: { local_x = norm_price; local_y = 0.0; }
            case 4u: { local_x = norm_next_price; local_y = norm_volume; }
            case 5u: { local_x = norm_next_price; local_y = 0.0; }
            default: { local_x = 0.0; local_y = 0.0; }
        }
    } else {
        // Vertical step (connecting this level to previous)
        switch vertex_index {
            case 0u: { local_x = norm_price; local_y = norm_prev_volume; }
            case 1u: { local_x = norm_price; local_y = norm_volume; }
            case 2u: { local_x = norm_price + 0.002; local_y = norm_volume; }
            case 3u: { local_x = norm_price; local_y = norm_prev_volume; }
            case 4u: { local_x = norm_price + 0.002; local_y = norm_volume; }
            case 5u: { local_x = norm_price + 0.002; local_y = norm_prev_volume; }
            default: { local_x = 0.0; local_y = 0.0; }
        }
    }

    // Apply padding and convert to NDC
    let padding = 0.1;
    let ndc_x = (local_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (local_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vec2<f32>(local_x, local_y);

    // Set color based on bid/ask and fill/line
    if is_bid {
        out.color = select(colors.bid_line, colors.bid_fill, is_fill);
        out.side = 0u;
    } else {
        out.color = select(colors.ask_line, colors.ask_fill, is_fill);
        out.side = 1u;
    }

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // Hover effect
    if uniforms.pointer.x >= 0.0 {
        let padding = 0.1;
        let pointer_x = uniforms.pointer.x * (1.0 - 2.0 * padding) + padding;
        let mid_norm = normalize_price(colors.mid_price);

        // Highlight the side being hovered
        let hovering_bid = pointer_x < mid_norm && in.side == 0u;
        let hovering_ask = pointer_x >= mid_norm && in.side == 1u;

        if hovering_bid || hovering_ask {
            color = vec4<f32>(
                min(color.r * 1.15, 1.0),
                min(color.g * 1.15, 1.0),
                min(color.b * 1.15, 1.0),
                color.a
            );
        }
    }

    // Gradient effect (darker at bottom)
    let gradient = 0.7 + 0.3 * in.uv.y;
    color = vec4<f32>(color.rgb * gradient, color.a);

    // Premultiply alpha
    return vec4<f32>(color.rgb * color.a, color.a);
}
