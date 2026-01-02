// Common shader utilities for WaterUI chart components.
//
// This file provides shared functions for:
// - Easing (cubic bezier and spring)
// - SDF primitives for anti-aliased rendering
// - Color utilities
// - Coordinate transformations

// ============================================================================
// Constants
// ============================================================================

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;
const EPSILON: f32 = 0.0001;

// ============================================================================
// Easing Functions
// ============================================================================

// Linear interpolation (built-in mix is also available)
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}

fn lerp2(a: vec2<f32>, b: vec2<f32>, t: f32) -> vec2<f32> {
    return a + (b - a) * t;
}

fn lerp4(a: vec4<f32>, b: vec4<f32>, t: f32) -> vec4<f32> {
    return a + (b - a) * t;
}

// Cubic bezier easing.
// Control points: (0, 0) -> (x1, y1) -> (x2, y2) -> (1, 1)
fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    // Newton-Raphson iteration to find t for given x
    var guess = t;
    for (var i = 0u; i < 8u; i++) {
        let x = bezier_sample(guess, x1, x2) - t;
        if (abs(x) < EPSILON) {
            break;
        }
        let dx = bezier_derivative(guess, x1, x2);
        if (abs(dx) < EPSILON) {
            break;
        }
        guess = guess - x / dx;
    }
    guess = clamp(guess, 0.0, 1.0);
    return bezier_sample(guess, y1, y2);
}

fn bezier_sample(t: f32, p1: f32, p2: f32) -> f32 {
    // B(t) = 3(1-t)²t·P1 + 3(1-t)t²·P2 + t³
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    return 3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3;
}

fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let t2 = t * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    return 3.0 * mt2 * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t2 * (1.0 - p2);
}

// Standard easing curves (CSS-compatible control points)
fn ease_linear(t: f32) -> f32 {
    return t;
}

fn ease_in(t: f32) -> f32 {
    return cubic_bezier(t, 0.42, 0.0, 1.0, 1.0);
}

fn ease_out(t: f32) -> f32 {
    return cubic_bezier(t, 0.0, 0.0, 0.58, 1.0);
}

fn ease_in_out(t: f32) -> f32 {
    return cubic_bezier(t, 0.42, 0.0, 0.58, 1.0);
}

// Spring easing (damped harmonic oscillator)
// Can overshoot and oscillate before settling at 1.0
fn ease_spring(t: f32, stiffness: f32, damping: f32) -> f32 {
    if (t <= 0.0) {
        return 0.0;
    }
    if (t >= 1.0) {
        return 1.0;
    }

    let omega = sqrt(stiffness);
    let zeta = damping / (2.0 * omega);

    if (zeta >= 1.0) {
        // Critically damped or overdamped
        let decay = exp(-omega * zeta * t);
        return 1.0 - decay * (1.0 + omega * zeta * t);
    } else {
        // Underdamped - oscillates
        let omega_d = omega * sqrt(1.0 - zeta * zeta);
        let decay = exp(-zeta * omega * t);
        let cos_part = cos(omega_d * t);
        let sin_part = (zeta * omega / omega_d) * sin(omega_d * t);
        return 1.0 - decay * (cos_part + sin_part);
    }
}

// Apply easing based on type enum (matches ChartAnimation.easing)
// 0 = Linear, 1 = EaseIn, 2 = EaseOut, 3 = EaseInOut, 4 = Spring
fn apply_easing(t: f32, easing_type: u32) -> f32 {
    switch easing_type {
        case 0u: { return ease_linear(t); }
        case 1u: { return ease_in(t); }
        case 2u: { return ease_out(t); }
        case 3u: { return ease_in_out(t); }
        case 4u: { return ease_spring(t, 100.0, 10.0); } // Default spring params
        default: { return ease_in_out(t); }
    }
}

// ============================================================================
// SDF (Signed Distance Field) Primitives
// ============================================================================

// Rectangle SDF (centered at origin)
fn sdf_rect(p: vec2<f32>, half_size: vec2<f32>) -> f32 {
    let d = abs(p) - half_size;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// Rounded rectangle SDF
fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

// Circle SDF (centered at origin)
fn sdf_circle(p: vec2<f32>, radius: f32) -> f32 {
    return length(p) - radius;
}

// Line segment SDF
fn sdf_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ba = b - a;
    let pa = p - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Annular (ring) SDF
fn sdf_ring(p: vec2<f32>, inner_radius: f32, outer_radius: f32) -> f32 {
    let d = length(p);
    return max(inner_radius - d, d - outer_radius);
}

// Arc (pie slice) SDF
fn sdf_arc(p: vec2<f32>, start_angle: f32, end_angle: f32, inner_radius: f32, outer_radius: f32) -> f32 {
    let angle = atan2(p.y, p.x);
    let r = length(p);

    // Radial distance
    let radial_dist = max(inner_radius - r, r - outer_radius);

    // Angular distance (normalized to handle wrapping)
    var a = angle;
    if (a < start_angle) { a = a + TAU; }
    let mid = (start_angle + end_angle) * 0.5;
    let half_span = (end_angle - start_angle) * 0.5;

    let angular_dist = max(0.0, abs(a - mid) - half_span);
    let arc_dist = angular_dist * r; // Convert to linear distance

    return max(radial_dist, arc_dist);
}

// Anti-aliased fill from SDF
// Returns 1.0 inside, 0.0 outside, with smooth edge
fn sdf_fill(d: f32, edge_width: f32) -> f32 {
    return smoothstep(edge_width, -edge_width, d);
}

// Anti-aliased stroke from SDF
fn sdf_stroke(d: f32, stroke_width: f32, edge_width: f32) -> f32 {
    return smoothstep(stroke_width * 0.5 + edge_width, stroke_width * 0.5 - edge_width, abs(d));
}

/// Calculate anti-aliased coverage from signed distance
/// Uses screen-space derivatives for resolution-independent AA
/// This is the preferred method - auto-calculates AA width per pixel
fn sdf_coverage(dist: f32) -> f32 {
    let aa_width = fwidth(dist);
    return 1.0 - smoothstep(-aa_width, aa_width, dist);
}

/// Capsule SDF (line segment with thickness) - for anti-aliased lines
fn sdf_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, radius: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - radius;
}

// ============================================================================
// HDR Tone Mapping
// ============================================================================

/// ACES filmic tone mapping (sRGB approximation)
/// Maps HDR values to displayable SDR range while preserving color fidelity
fn tonemap_aces(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((color * (a * color + b)) / (color * (c * color + d) + e));
}

/// Reinhard tone mapping (simple, preserves highlights)
fn tonemap_reinhard(color: vec3<f32>) -> vec3<f32> {
    return color / (color + vec3<f32>(1.0));
}

/// Extended Reinhard with adjustable white point
fn tonemap_reinhard_extended(color: vec3<f32>, white_point: f32) -> vec3<f32> {
    let wp2 = white_point * white_point;
    return (color * (1.0 + color / wp2)) / (1.0 + color);
}

// ============================================================================
// Oklab Color Space (Perceptually Uniform)
// ============================================================================

/// Linear sRGB to Oklab
/// Oklab provides perceptually uniform color interpolation
fn srgb_to_oklab(c: vec3<f32>) -> vec3<f32> {
    let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
    let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
    let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;

    let l_ = pow(l, 1.0 / 3.0);
    let m_ = pow(m, 1.0 / 3.0);
    let s_ = pow(s, 1.0 / 3.0);

    return vec3<f32>(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_
    );
}

/// Oklab to linear sRGB
fn oklab_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let l_ = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
    let m_ = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
    let s_ = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    return vec3<f32>(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    );
}

/// Mix two colors in Oklab space for perceptually uniform blending
fn mix_oklab(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    let oklab_a = srgb_to_oklab(a);
    let oklab_b = srgb_to_oklab(b);
    let mixed = mix(oklab_a, oklab_b, t);
    return oklab_to_srgb(mixed);
}

// ============================================================================
// Color Utilities
// ============================================================================

// sRGB to linear conversion
fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn srgb_to_linear3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b)
    );
}

// Linear to sRGB conversion
fn linear_to_srgb(c: f32) -> f32 {
    if (c <= 0.0031308) {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn linear_to_srgb3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        linear_to_srgb(c.r),
        linear_to_srgb(c.g),
        linear_to_srgb(c.b)
    );
}

// Premultiply alpha
fn premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Blend (over operator, assumes premultiplied alpha)
fn blend_over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    return src + dst * (1.0 - src.a);
}

// Color interpolation in linear space (better for gradients)
fn lerp_color(a: vec4<f32>, b: vec4<f32>, t: f32) -> vec4<f32> {
    // Convert to linear, interpolate, convert back
    let a_linear = vec4<f32>(srgb_to_linear3(a.rgb), a.a);
    let b_linear = vec4<f32>(srgb_to_linear3(b.rgb), b.a);
    let result = lerp4(a_linear, b_linear, t);
    return vec4<f32>(linear_to_srgb3(result.rgb), result.a);
}

// Sample color from a gradient (stops are in uniform buffer)
fn sample_gradient(t: f32, stop_count: u32, stops: array<vec4<f32>, 8>) -> vec4<f32> {
    if (stop_count == 0u) {
        return vec4<f32>(1.0);
    }
    if (stop_count == 1u) {
        return stops[0];
    }

    // Clamp to gradient range
    let clamped_t = clamp(t, 0.0, 1.0);

    // Find the two stops to interpolate between
    // Assumes stops are sorted by position (stored in .a)
    var lower = 0u;
    for (var i = 0u; i < stop_count - 1u; i++) {
        if (stops[i + 1u].a <= clamped_t) {
            lower = i + 1u;
        }
    }
    let upper = min(lower + 1u, stop_count - 1u);

    // Calculate interpolation factor
    let range = stops[upper].a - stops[lower].a;
    var local_t = 0.0;
    if (range > EPSILON) {
        local_t = (clamped_t - stops[lower].a) / range;
    }

    return lerp_color(stops[lower], stops[upper], local_t);
}

// ============================================================================
// Coordinate Utilities
// ============================================================================

// Transform from normalized device coordinates to data coordinates
fn ndc_to_data(ndc: vec2<f32>, bounds: vec4<f32>) -> vec2<f32> {
    // bounds = [min_x, max_x, min_y, max_y]
    let data_x = bounds.x + ndc.x * (bounds.y - bounds.x);
    let data_y = bounds.z + (1.0 - ndc.y) * (bounds.w - bounds.z); // Flip Y
    return vec2<f32>(data_x, data_y);
}

// Transform from data coordinates to normalized device coordinates
fn data_to_ndc(data: vec2<f32>, bounds: vec4<f32>) -> vec2<f32> {
    let ndc_x = (data.x - bounds.x) / (bounds.y - bounds.x);
    let ndc_y = 1.0 - (data.y - bounds.z) / (bounds.w - bounds.z); // Flip Y
    return vec2<f32>(ndc_x, ndc_y);
}

// Get pixel size for anti-aliasing calculations
fn pixel_size(viewport: vec4<f32>) -> vec2<f32> {
    // viewport = [width, height, 1/width, 1/height]
    return vec2<f32>(viewport.z, viewport.w);
}

// ============================================================================
// Chart-specific Helpers
// ============================================================================

// Check if a point is within the chart viewport (normalized 0-1 space)
fn in_viewport(p: vec2<f32>) -> bool {
    return p.x >= 0.0 && p.x <= 1.0 && p.y >= 0.0 && p.y <= 1.0;
}

// Calculate bar position (0-indexed)
fn bar_position(index: u32, bar_count: u32, bar_width: f32, gap: f32) -> f32 {
    let total_width = f32(bar_count) * bar_width + f32(bar_count - 1u) * gap;
    let start = 0.5 - total_width * 0.5;
    return start + f32(index) * (bar_width + gap) + bar_width * 0.5;
}

// Calculate candlestick body color based on open/close
fn candle_color(open: f32, close: f32, bullish: vec4<f32>, bearish: vec4<f32>) -> vec4<f32> {
    if (close >= open) {
        return bullish;
    }
    return bearish;
}
