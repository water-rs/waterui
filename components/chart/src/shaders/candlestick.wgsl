// Candlestick (K-line) chart shader.
//
// Renders OHLC candles using instanced rendering. Each instance draws:
// - A vertical wick line from low to high
// - A body rectangle from open to close
//
// Supports:
// - Animation interpolation between previous and current data
// - Entry animation (candles grow from mid-point)
// - Hover highlighting
// - Bullish (green) vs bearish (red) coloring

// Uniforms
struct ChartUniforms {
    // Viewport: [width, height, 1/width, 1/height]
    viewport: vec4<f32>,
    // Data bounds: [min_x, max_x, min_y, max_y]
    bounds: vec4<f32>,
    // Animation: [time, progress, easing, entry_active]
    animation: vec4<f32>,
    // Pointer: [x, y, pressed, 0] - normalized coordinates, -1 if not hovering
    pointer: vec4<f32>,
}

// Candle data point
struct CandleData {
    timestamp: f32,
    open: f32,
    high: f32,
    low: f32,
    close: f32,
    volume: f32,
    _pad0: f32,
    _pad1: f32,
}

// Colors for bullish/bearish candles
struct CandleColors {
    bullish: vec4<f32>,
    bearish: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: ChartUniforms;
@group(0) @binding(1) var<uniform> colors: CandleColors;
@group(0) @binding(2) var<storage, read> current_data: array<CandleData>;
@group(0) @binding(3) var<storage, read> previous_data: array<CandleData>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_uv: vec2<f32>,       // UV within the current element
    @location(2) @interpolate(flat) candle_index: u32,
    @location(3) @interpolate(flat) element_type: u32,  // 0 = body, 1 = wick
}

// Interpolate candle data
fn mix_candle(prev: CandleData, curr: CandleData, t: f32) -> CandleData {
    var result: CandleData;
    result.timestamp = mix(prev.timestamp, curr.timestamp, t);
    result.open = mix(prev.open, curr.open, t);
    result.high = mix(prev.high, curr.high, t);
    result.low = mix(prev.low, curr.low, t);
    result.close = mix(prev.close, curr.close, t);
    result.volume = mix(prev.volume, curr.volume, t);
    return result;
}

// Normalize Y coordinate to [0, 1] range
fn normalize_y(y: f32) -> f32 {
    let y_range = uniforms.bounds.w - uniforms.bounds.z;
    return select(
        (y - uniforms.bounds.z) / y_range,
        0.5,
        y_range <= 0.0
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    let data_count = arrayLength(&current_data);

    // Each candle has 2 elements: body (6 vertices) + wick (6 vertices) = 12 vertices per candle
    let candle_index = instance_index / 2u;
    let element_type = instance_index % 2u;  // 0 = body, 1 = wick

    if candle_index >= data_count {
        out.position = vec4<f32>(0.0);
        return out;
    }

    // Get current and previous candle data
    let curr = current_data[candle_index];
    let prev = previous_data[candle_index];

    // Apply animation interpolation
    let progress = clamp(uniforms.animation.y, 0.0, 1.0);
    let eased_progress = apply_easing(progress, u32(uniforms.animation.z));
    let candle = mix_candle(prev, curr, eased_progress);

    // Calculate candle dimensions
    let candle_count = f32(data_count);
    let candle_width = 0.8 / candle_count;
    let gap = 0.2 / (candle_count + 1.0);
    let wick_width = candle_width * 0.1;  // Wick is 10% of body width

    // Calculate candle X position (centered)
    let candle_center_x = gap + (candle_width + gap) * f32(candle_index) + candle_width * 0.5;

    // Normalize OHLC values
    let open_y = normalize_y(candle.open);
    let high_y = normalize_y(candle.high);
    let low_y = normalize_y(candle.low);
    let close_y = normalize_y(candle.close);

    // Entry animation: grow from mid-point
    var body_top: f32;
    var body_bottom: f32;
    var wick_top: f32;
    var wick_bottom: f32;

    if uniforms.animation.w > 0.5 {
        let mid_y = (open_y + close_y) * 0.5;
        let body_height = abs(close_y - open_y) * eased_progress;
        let wick_extent_top = (high_y - max(open_y, close_y)) * eased_progress;
        let wick_extent_bottom = (min(open_y, close_y) - low_y) * eased_progress;

        body_top = mid_y + body_height * 0.5;
        body_bottom = mid_y - body_height * 0.5;
        wick_top = body_top + wick_extent_top;
        wick_bottom = body_bottom - wick_extent_bottom;
    } else {
        body_top = max(open_y, close_y);
        body_bottom = min(open_y, close_y);
        wick_top = high_y;
        wick_bottom = low_y;
    }

    // Generate quad vertices (6 vertices = 2 triangles)
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

    var chart_x: f32;
    var chart_y: f32;
    var width: f32;

    if element_type == 0u {
        // Body
        width = candle_width;
        chart_x = candle_center_x + local_x * width;
        chart_y = body_bottom + local_y * (body_top - body_bottom);
    } else {
        // Wick
        width = wick_width;
        chart_x = candle_center_x + local_x * width;
        chart_y = wick_bottom + local_y * (wick_top - wick_bottom);
    }

    // Map to NDC with padding
    let padding = 0.1;
    let ndc_x = (chart_x * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;
    let ndc_y = (chart_y * (1.0 - 2.0 * padding) + padding) * 2.0 - 1.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // Set color based on bullish/bearish
    out.color = candle_color(candle.open, candle.close, colors.bullish, colors.bearish);
    out.local_uv = uv;
    out.candle_index = candle_index;
    out.element_type = element_type;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;

    // SDF for rounded corners on body, sharp on wick
    let centered = (in.local_uv - 0.5) * 2.0;  // [-1, 1]
    var aa: f32;

    if in.element_type == 0u {
        // Body: rounded rectangle
        let corner_radius = 0.15;
        let dist = sdf_rounded_rect(centered, vec2<f32>(1.0 - corner_radius, 1.0 - corner_radius), corner_radius);
        aa = sdf_coverage(dist);
    } else {
        // Wick: sharp rectangle
        let dist = sdf_rect(centered, vec2<f32>(1.0, 1.0));
        aa = sdf_coverage(dist);
    }

    // Early discard for fully transparent pixels
    if aa < 0.001 {
        discard;
    }

    // Check if this candle is being hovered
    if uniforms.pointer.x >= 0.0 {
        let data_count = f32(arrayLength(&current_data));
        let candle_width = 0.8 / data_count;
        let gap = 0.2 / (data_count + 1.0);
        let candle_center_x = gap + (candle_width + gap) * f32(in.candle_index) + candle_width * 0.5;

        let padding = 0.1;
        let pointer_x = uniforms.pointer.x * (1.0 - 2.0 * padding) + padding;

        let candle_left = candle_center_x - candle_width * 0.5;
        let candle_right = candle_center_x + candle_width * 0.5;

        if pointer_x >= candle_left && pointer_x <= candle_right {
            // Highlight on hover
            color = vec4<f32>(
                min(color.r * 1.2, 1.0),
                min(color.g * 1.2, 1.0),
                min(color.b * 1.2, 1.0),
                color.a
            );
        }
    }

    // Subtle gradient on body (lighter at top)
    if in.element_type == 0u {
        let gradient = 0.85 + 0.15 * in.local_uv.y;
        color = vec4<f32>(color.rgb * gradient, color.a);
    }

    // Apply alpha and anti-aliasing
    let final_alpha = color.a * aa;

    // Premultiply alpha for correct blending
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
