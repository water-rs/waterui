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
@group(0) @binding(4) var aux_texture_2: texture_2d<f32>;
@group(0) @binding(5) var<uniform> uniforms: Uniforms;

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

fn sample_aux2(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(aux_texture_2, input_sampler, uv, 0.0);
}

fn blend_overlay(base: vec3<f32>, top: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * base * top;
    let high = 1.0 - 2.0 * (1.0 - base) * (1.0 - top);
    let mask = step(vec3<f32>(0.5), base);
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
        default: {
            return base;
        }
    }
}
