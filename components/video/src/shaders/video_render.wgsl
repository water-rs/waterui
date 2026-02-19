struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = uv;
    return output;
}

@group(0) @binding(0) var y_texture: texture_2d<f32>;
@group(0) @binding(1) var uv_texture: texture_2d<f32>;
@group(0) @binding(2) var video_sampler: sampler;

struct ColorParams {
    matrix_mode: u32,
    range_mode: u32,
    primaries_mode: u32,
    transfer_mode: u32,
    target_mode: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
}

@group(0) @binding(3) var<uniform> color_params: ColorParams;

const MATRIX_BT709: u32 = 0u;
const MATRIX_BT601: u32 = 1u;
const MATRIX_BT2020: u32 = 2u;

const RANGE_LIMITED: u32 = 0u;

const PRIMARIES_BT709: u32 = 0u;
const PRIMARIES_BT601: u32 = 1u;
const PRIMARIES_DISPLAY_P3: u32 = 2u;
const PRIMARIES_BT2020: u32 = 3u;

const TRANSFER_SDR: u32 = 0u;
const TRANSFER_PQ: u32 = 1u;
const TRANSFER_HLG: u32 = 2u;

const TARGET_GAMMA_SDR: u32 = 0u;
const TARGET_LINEAR_SDR: u32 = 1u;
const TARGET_LINEAR_HDR: u32 = 2u;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn bt709_to_linear(c: f32) -> f32 {
    if c < 0.081 {
        return c / 4.5;
    }
    return pow((c + 0.099) / 1.099, 1.0 / 0.45);
}

fn linear_to_bt709(c: f32) -> f32 {
    if c < 0.018 {
        return c * 4.5;
    }
    return 1.099 * pow(c, 0.45) - 0.099;
}

fn pq_to_linear(value: f32) -> f32 {
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 32.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 128.0;
    let c3 = 2392.0 / 128.0;

    let v = clamp(value, 0.0, 1.0);
    let v_pow = pow(v, 1.0 / m2);
    let numerator = max(v_pow - c1, 0.0);
    let denominator = max(c2 - c3 * v_pow, 1e-6);
    let absolute_nits = 10000.0 * pow(numerator / denominator, 1.0 / m1);

    // Normalize to an SDR reference white of 100 nits.
    return absolute_nits / 100.0;
}

fn hlg_to_linear(value: f32) -> f32 {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let e = clamp(value, 0.0, 1.0);
    var scene_linear = 0.0;
    if e <= 0.5 {
        scene_linear = (e * e) / 3.0;
    } else {
        scene_linear = (exp((e - c) / a) + b) / 12.0;
    }

    // Keep typical HLG highlights above SDR white.
    return scene_linear * 12.0;
}

fn decode_transfer_to_linear(rgb: vec3<f32>, transfer_mode: u32) -> vec3<f32> {
    if transfer_mode == TRANSFER_PQ {
        return vec3<f32>(
            pq_to_linear(rgb.r),
            pq_to_linear(rgb.g),
            pq_to_linear(rgb.b),
        );
    }
    if transfer_mode == TRANSFER_HLG {
        return vec3<f32>(
            hlg_to_linear(rgb.r),
            hlg_to_linear(rgb.g),
            hlg_to_linear(rgb.b),
        );
    }
    return vec3<f32>(
        bt709_to_linear(rgb.r),
        bt709_to_linear(rgb.g),
        bt709_to_linear(rgb.b),
    );
}

fn convert_primaries_to_srgb(linear_rgb: vec3<f32>, primaries_mode: u32) -> vec3<f32> {
    if primaries_mode == PRIMARIES_BT2020 {
        return vec3<f32>(
            1.6605 * linear_rgb.r - 0.5876 * linear_rgb.g - 0.0728 * linear_rgb.b,
            -0.1246 * linear_rgb.r + 1.1329 * linear_rgb.g - 0.0083 * linear_rgb.b,
            -0.0182 * linear_rgb.r - 0.1006 * linear_rgb.g + 1.1188 * linear_rgb.b,
        );
    }

    if primaries_mode == PRIMARIES_DISPLAY_P3 {
        return vec3<f32>(
            1.2249 * linear_rgb.r - 0.2247 * linear_rgb.g - 0.0002 * linear_rgb.b,
            -0.0420 * linear_rgb.r + 1.0419 * linear_rgb.g + 0.0001 * linear_rgb.b,
            -0.0197 * linear_rgb.r - 0.0786 * linear_rgb.g + 1.0983 * linear_rgb.b,
        );
    }

    return linear_rgb;
}

fn tone_map_hdr_to_sdr(linear_rgb: vec3<f32>) -> vec3<f32> {
    let safe = max(linear_rgb, vec3<f32>(0.0));
    return safe / (safe + vec3<f32>(1.0));
}

fn yuv_to_gamma_rgb(y_sample: f32, uv_sample: vec2<f32>) -> vec3<f32> {
    var y = y_sample;
    var u = uv_sample.x - 0.5;
    var v = uv_sample.y - 0.5;

    if color_params.range_mode == RANGE_LIMITED {
        y = max(0.0, y - (16.0 / 255.0)) * (255.0 / 219.0);
        let chroma_scale = 255.0 / 224.0;
        u = u * chroma_scale;
        v = v * chroma_scale;
    }

    var r = 0.0;
    var g = 0.0;
    var b = 0.0;

    if color_params.matrix_mode == MATRIX_BT601 {
        r = y + 1.402 * v;
        g = y - 0.344136 * u - 0.714136 * v;
        b = y + 1.772 * u;
    } else if color_params.matrix_mode == MATRIX_BT2020 {
        r = y + 1.4746 * v;
        g = y - 0.164553 * u - 0.571353 * v;
        b = y + 1.8814 * u;
    } else {
        // BT.709
        r = y + 1.5748 * v;
        g = y - 0.187324 * u - 0.468124 * v;
        b = y + 1.8556 * u;
    }

    return clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let y = textureSample(y_texture, video_sampler, input.uv).r;
    let uv = textureSample(uv_texture, video_sampler, input.uv).rg;
    let gamma_rgb = yuv_to_gamma_rgb(y, uv);

    var linear_rgb = decode_transfer_to_linear(gamma_rgb, color_params.transfer_mode);
    linear_rgb = convert_primaries_to_srgb(linear_rgb, color_params.primaries_mode);

    if color_params.target_mode == TARGET_LINEAR_HDR {
        return vec4<f32>(max(linear_rgb, vec3<f32>(0.0)), 1.0);
    }

    if color_params.transfer_mode != TRANSFER_SDR {
        linear_rgb = tone_map_hdr_to_sdr(linear_rgb);
    }

    let clamped_linear = clamp(linear_rgb, vec3<f32>(0.0), vec3<f32>(1.0));

    if color_params.target_mode == TARGET_LINEAR_SDR {
        return vec4<f32>(clamped_linear, 1.0);
    }

    let gamma_sdr = vec3<f32>(
        linear_to_bt709(clamped_linear.r),
        linear_to_bt709(clamped_linear.g),
        linear_to_bt709(clamped_linear.b),
    );
    if color_params.target_mode == TARGET_GAMMA_SDR {
        return vec4<f32>(gamma_sdr, 1.0);
    }

    return vec4<f32>(clamped_linear, 1.0);
}
