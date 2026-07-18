struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );

    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

struct Uniforms {
    output_size: vec2<f32>,
    _pad0: vec2<f32>,
    op0: vec4<f32>,
    op1: vec4<f32>,
    op2: vec4<f32>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var aux_texture_0: texture_2d<f32>;
@group(0) @binding(3) var aux_texture_1: texture_2d<f32>;
@group(0) @binding(4) var<uniform> uniforms: Uniforms;

fn param(index: u32) -> f32 {
    switch index {
        case 0u: {
            return uniforms.op0.y;
        }
        case 1u: {
            return uniforms.op0.z;
        }
        case 2u: {
            return uniforms.op0.w;
        }
        case 3u: {
            return uniforms.op1.x;
        }
        case 4u: {
            return uniforms.op1.y;
        }
        case 5u: {
            return uniforms.op1.z;
        }
        case 6u: {
            return uniforms.op1.w;
        }
        default: {
            return uniforms.op2.x;
        }
    }
}

fn op_mode() -> u32 {
    return u32(uniforms.op0.x + 0.5);
}

fn sample_main(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(input_texture, input_sampler, uv, 0.0);
}

fn sample_aux0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(aux_texture_0, input_sampler, uv, 0.0);
}

fn sample_aux1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(aux_texture_1, input_sampler, uv, 0.0);
}

fn blend_overlay(base: vec3<f32>, top: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * base * top;
    let high = 1.0 - 2.0 * (1.0 - base) * (1.0 - top);
    let mask = step(vec3<f32>(0.5), base);
    return mix(low, high, mask);
}

// Helpers used by the HSL-based composite blend modes (Hue/Saturation/
// Color/Luminosity). These mirror the algorithm sketched in the SVG/PDF
// compositing spec, working in HSL rather than HSY for simplicity.

fn rgb_to_hsl_max_min(rgb: vec3<f32>) -> vec3<f32> {
    let max_c = max(max(rgb.r, rgb.g), rgb.b);
    let min_c = min(min(rgb.r, rgb.g), rgb.b);
    let l = (max_c + min_c) * 0.5;
    if max_c == min_c {
        return vec3<f32>(0.0, 0.0, l);
    }
    let d = max_c - min_c;
    let s = select(d / (2.0 - max_c - min_c), d / (max_c + min_c), l > 0.5);
    var h: f32;
    if max_c == rgb.r {
        h = (rgb.g - rgb.b) / d + select(0.0, 6.0, rgb.g < rgb.b);
    } else if max_c == rgb.g {
        h = (rgb.b - rgb.r) / d + 2.0;
    } else {
        h = (rgb.r - rgb.g) / d + 4.0;
    }
    return vec3<f32>(h / 6.0, s, l);
}

fn hue_to_rgb_helper(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in;
    if t < 0.0 { t = t + 1.0; }
    if t > 1.0 { t = t - 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 0.5 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb_local(hsl: vec3<f32>) -> vec3<f32> {
    if hsl.y == 0.0 {
        return vec3<f32>(hsl.z);
    }
    let q = select(hsl.z + hsl.y - hsl.z * hsl.y, hsl.z * (1.0 + hsl.y), hsl.z < 0.5);
    let p = 2.0 * hsl.z - q;
    return vec3<f32>(
        hue_to_rgb_helper(p, q, hsl.x + 1.0 / 3.0),
        hue_to_rgb_helper(p, q, hsl.x),
        hue_to_rgb_helper(p, q, hsl.x - 1.0 / 3.0),
    );
}

fn blend_hsl(base: vec3<f32>, top: vec3<f32>, take_h_top: bool, take_s_top: bool, take_l_top: bool) -> vec3<f32> {
    let base_hsl = rgb_to_hsl_max_min(base);
    let top_hsl = rgb_to_hsl_max_min(top);
    let h = select(base_hsl.x, top_hsl.x, take_h_top);
    let s = select(base_hsl.y, top_hsl.y, take_s_top);
    let l = select(base_hsl.z, top_hsl.z, take_l_top);
    return hsl_to_rgb_local(vec3<f32>(h, s, l));
}

fn blend_soft_light(base: vec3<f32>, top: vec3<f32>) -> vec3<f32> {
    let low = base - (1.0 - 2.0 * top) * base * (1.0 - base);
    let high = base + (2.0 * top - 1.0) * (sqrt(max(base, vec3<f32>(0.0))) - base);
    let mask = step(vec3<f32>(0.5), top);
    return mix(low, high, mask);
}

fn blend_color(base: vec3<f32>, top: vec3<f32>, mode: u32) -> vec3<f32> {
    switch mode {
        case 1u: {
            return base * top;
        }
        case 2u: {
            return 1.0 - (1.0 - base) * (1.0 - top);
        }
        case 3u: {
            return blend_overlay(base, top);
        }
        case 4u: {
            return min(base, top);
        }
        case 5u: {
            return max(base, top);
        }
        case 6u: {
            return blend_soft_light(base, top);
        }
        case 7u: {
            return blend_overlay(top, base);
        }
        case 8u: {
            return abs(base - top);
        }
        case 9u: {
            return base + top - 2.0 * base * top;
        }
        case 10u: {
            return base / max(vec3<f32>(1.0) - top, vec3<f32>(0.0001));
        }
        case 11u: {
            return 1.0 - (1.0 - base) / max(top, vec3<f32>(0.0001));
        }
        case 12u: {
            // Hue: hue from top, saturation+luminance from base.
            return blend_hsl(base, top, true, false, false);
        }
        case 13u: {
            // Saturation: saturation from top, hue+luminance from base.
            return blend_hsl(base, top, false, true, false);
        }
        case 14u: {
            // Color: hue+saturation from top, luminance from base.
            return blend_hsl(base, top, true, true, false);
        }
        case 15u: {
            // Luminosity: luminance from top, hue+saturation from base.
            return blend_hsl(base, top, false, false, true);
        }
        default: {
            return top;
        }
    }
}

fn box_blur(uv: vec2<f32>, radius: i32) -> vec4<f32> {
    if radius <= 0 {
        return sample_main(uv);
    }

    let texel = 1.0 / uniforms.output_size;
    var sum = vec4<f32>(0.0);
    var count = 0.0;
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_uv = uv + vec2<f32>(f32(x), f32(y)) * texel;
            sum += sample_main(sample_uv);
            count += 1.0;
        }
    }
    return sum / max(count, 1.0);
}

fn guided_smooth_sample(uv: vec2<f32>, radius: i32, sigma: f32) -> vec4<f32> {
    if radius <= 0 {
        return sample_main(uv);
    }

    let center_guide = sample_aux0(uv).rgb;
    let texel = 1.0 / uniforms.output_size;
    let inv_sigma = 1.0 / max(sigma, 0.0001);

    var weighted_sum = vec4<f32>(0.0);
    var weight_total = 0.0;

    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_uv = uv + vec2<f32>(f32(x), f32(y)) * texel;
            let guide_rgb = sample_aux0(sample_uv).rgb;
            let diff = length(guide_rgb - center_guide);
            let weight = exp(-diff * inv_sigma);
            weighted_sum += sample_main(sample_uv) * weight;
            weight_total += weight;
        }
    }

    return weighted_sum / max(weight_total, 0.0001);
}

fn depth_aware_blur_sample(uv: vec2<f32>, radius: i32, center_depth: f32) -> vec4<f32> {
    if radius <= 0 {
        return sample_main(uv);
    }

    let texel = 1.0 / uniforms.output_size;
    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;

    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_uv = uv + vec2<f32>(f32(x), f32(y)) * texel;
            let sample_depth = sample_aux0(sample_uv).r;
            let depth_delta = abs(sample_depth - center_depth);
            let depth_weight = 1.0 - smoothstep(0.0, 0.25, depth_delta);
            let color = sample_main(sample_uv);
            sum += color * depth_weight;
            total_weight += depth_weight;
        }
    }

    return sum / max(total_weight, 0.0001);
}

fn lut_texel(r: u32, g: u32, b: u32, lut_size: u32) -> vec3<f32> {
    let dims = textureDimensions(aux_texture_0, 0);
    let x = min(b * lut_size + r, dims.x - 1u);
    let y = min(g, dims.y - 1u);
    return textureLoad(aux_texture_0, vec2<i32>(i32(x), i32(y)), 0).rgb;
}

fn sample_lut_strip(color: vec3<f32>, lut_size: u32) -> vec3<f32> {
    let clamped = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let size_f = f32(lut_size);
    let grid = clamped * (size_f - 1.0);

    let r0 = u32(floor(grid.r));
    let g0 = u32(floor(grid.g));
    let b0 = u32(floor(grid.b));

    let r1 = min(r0 + 1u, lut_size - 1u);
    let g1 = min(g0 + 1u, lut_size - 1u);
    let b1 = min(b0 + 1u, lut_size - 1u);

    let fr = grid.r - f32(r0);
    let fg = grid.g - f32(g0);
    let fb = grid.b - f32(b0);

    let c000 = lut_texel(r0, g0, b0, lut_size);
    let c100 = lut_texel(r1, g0, b0, lut_size);
    let c010 = lut_texel(r0, g1, b0, lut_size);
    let c110 = lut_texel(r1, g1, b0, lut_size);
    let c001 = lut_texel(r0, g0, b1, lut_size);
    let c101 = lut_texel(r1, g0, b1, lut_size);
    let c011 = lut_texel(r0, g1, b1, lut_size);
    let c111 = lut_texel(r1, g1, b1, lut_size);

    let c00 = mix(c000, c100, fr);
    let c10 = mix(c010, c110, fr);
    let c01 = mix(c001, c101, fr);
    let c11 = mix(c011, c111, fr);
    let c0 = mix(c00, c10, fg);
    let c1 = mix(c01, c11, fg);
    return mix(c0, c1, fb);
}

fn apply_swipe_transition(uv: vec2<f32>, progress: f32, softness: f32, direction: u32) -> f32 {
    var edge = uv.x;
    switch direction {
        case 1u: {
            edge = 1.0 - uv.x;
        }
        case 2u: {
            edge = uv.y;
        }
        case 3u: {
            edge = 1.0 - uv.y;
        }
        default: {}
    }
    return smoothstep(progress - softness, progress + softness, edge);
}

fn apply_radial_transition(
    uv: vec2<f32>,
    progress: f32,
    softness: f32,
    center: vec2<f32>,
) -> f32 {
    let radius = distance(uv, center) * 1.41421356;
    return smoothstep(progress - softness, progress + softness, radius);
}

fn apply_tone_curve_channel(
    x: f32,
    shadows: f32,
    midtones: f32,
    highlights: f32,
    gamma: f32,
) -> f32 {
    let g = pow(clamp(x, 0.0, 1.0), 1.0 / max(gamma, 0.001));
    let shadow_weight = (1.0 - g) * (1.0 - g);
    let highlight_weight = g * g;
    let mid_weight = 1.0 - abs(g * 2.0 - 1.0);
    let curved = g
        + shadows * shadow_weight
        + midtones * mid_weight
        + highlights * highlight_weight;
    return clamp(curved, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let base = sample_main(uv);
    let mode = op_mode();

    switch mode {
        case 0u: {
            let blend_mode = u32(param(0u) + 0.5);
            let amount = clamp(param(1u), 0.0, 1.0);
            let src = sample_aux0(uv);
            let blended = blend_color(base.rgb, src.rgb, blend_mode);
            return vec4<f32>(mix(base.rgb, blended, amount), base.a);
        }
        case 1u: {
            let radius = i32(round(max(param(0u), 0.0)));
            let strength = clamp(param(1u), 0.0, 1.0);
            let mask = clamp(sample_aux0(uv).r, 0.0, 1.0);
            let blurred = box_blur(uv, radius);
            return mix(base, blurred, mask * strength);
        }
        case 2u: {
            let progress = clamp(param(0u), 0.0, 1.0);
            let softness = max(param(1u), 0.001);
            let edge = smoothstep(progress - softness, progress + softness, uv.x);
            let target_color = sample_aux0(uv);
            return mix(base, target_color, edge);
        }
        case 3u: {
            let scale = vec2<f32>(param(0u), param(1u));
            let displacement = sample_aux0(uv).rg * 2.0 - vec2<f32>(1.0);
            let warped_uv = uv + displacement * scale / uniforms.output_size;
            return sample_main(warped_uv);
        }
        case 4u: {
            let radius = i32(round(max(param(0u), 0.0)));
            let sigma = max(param(1u), 0.0001);
            let amount = clamp(param(2u), 0.0, 1.0);
            let smooth_color = guided_smooth_sample(uv, radius, sigma);
            return mix(base, smooth_color, amount);
        }
        case 5u: {
            let focus_depth = clamp(param(0u), 0.0, 1.0);
            let aperture = max(param(1u), 0.0);
            let max_radius = max(param(2u), 0.0);
            let depth = clamp(sample_aux0(uv).r, 0.0, 1.0);
            let coc = abs(depth - focus_depth) * aperture * max_radius;
            let radius = i32(round(coc));
            let blurred = depth_aware_blur_sample(uv, radius, depth);
            let mix_t = clamp(coc / max(max_radius, 0.0001), 0.0, 1.0);
            return mix(base, blurred, mix_t);
        }
        case 6u: {
            let history_weight = clamp(param(0u), 0.0, 0.99);
            let motion = sample_aux1(uv).rg * 2.0 - vec2<f32>(1.0);
            let history_uv = uv - motion / uniforms.output_size;
            let history = sample_aux0(history_uv);
            return mix(base, history, history_weight);
        }
        case 7u: {
            let edge_softness = max(param(0u), 0.0001);
            let matte = clamp(sample_aux0(uv).r, 0.0, 1.0);
            let fg_alpha = smoothstep(0.5 - edge_softness, 0.5 + edge_softness, matte);
            let background = sample_aux1(uv);
            return mix(background, base, fg_alpha);
        }
        case 8u: {
            let lut_size = max(u32(round(param(0u))), 2u);
            let intensity = clamp(param(1u), 0.0, 1.0);
            let graded = sample_lut_strip(base.rgb, lut_size);
            return vec4<f32>(mix(base.rgb, graded, intensity), base.a);
        }
        case 9u: {
            let shadows = param(0u);
            let midtones = param(1u);
            let highlights = param(2u);
            let gamma = param(3u);
            let amount = clamp(param(4u), 0.0, 1.0);
            let curved = vec3<f32>(
                apply_tone_curve_channel(base.r, shadows, midtones, highlights, gamma),
                apply_tone_curve_channel(base.g, shadows, midtones, highlights, gamma),
                apply_tone_curve_channel(base.b, shadows, midtones, highlights, gamma),
            );
            return vec4<f32>(mix(base.rgb, curved, amount), base.a);
        }
        case 10u: {
            let progress = clamp(param(0u), 0.0, 1.0);
            let softness = max(param(1u), 0.001);
            let direction = u32(param(2u) + 0.5);
            let edge = apply_swipe_transition(uv, progress, softness, direction);
            let target_color = sample_aux0(uv);
            return mix(base, target_color, edge);
        }
        case 11u: {
            let progress = clamp(param(0u), 0.0, 1.0);
            let softness = max(param(1u), 0.001);
            let center = vec2<f32>(param(2u), param(3u));
            let radius = distance(uv, center);
            let edge = smoothstep(progress - softness, progress + softness, radius * 1.41421356);
            let target_color = sample_aux0(uv);
            return mix(base, target_color, edge);
        }
        case 12u: {
            let progress = clamp(param(0u), 0.0, 1.0);
            let amount = max(param(1u), 0.0);
            let center = vec2<f32>(param(2u), param(3u));
            let to_center = center - uv;
            let source_uv = uv - to_center * amount * (1.0 - progress);
            let target_uv = uv + to_center * amount * progress;
            let source = sample_main(source_uv);
            let target_color = sample_aux0(target_uv);
            return mix(source, target_color, progress);
        }
        case 13u: {
            let progress = clamp(param(0u), 0.0, 1.0);
            let scale = max(param(1u), 0.0);
            let displacement = sample_aux1(uv).rg * 2.0 - vec2<f32>(1.0);
            let source_uv = uv - displacement * scale * progress / uniforms.output_size;
            let target_uv = uv + displacement * scale * (1.0 - progress) / uniforms.output_size;
            let source = sample_main(source_uv);
            let target_color = sample_aux0(target_uv);
            return mix(source, target_color, progress);
        }
        default: {
            return base;
        }
    }
}
