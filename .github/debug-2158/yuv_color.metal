// language: metal1.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct ColorParams {
    uint matrix_mode;
    uint range_mode;
    uint primaries_mode;
    uint transfer_mode;
    uint target_mode;
    uint sample_mode;
    float max_content_light_nits;
    uint _padding1_;
};
constant uint MATRIX_BT709_ = 0u;
constant uint MATRIX_BT601_ = 1u;
constant uint MATRIX_BT2020_ = 2u;
constant uint MATRIX_BT2020_CONSTANT_LUMINANCE = 3u;
constant uint RANGE_LIMITED = 0u;
constant uint SAMPLE_NV12_ = 0u;
constant uint SAMPLE_P010_ = 1u;
constant uint PRIMARIES_BT709_ = 0u;
constant uint PRIMARIES_BT601_ = 1u;
constant uint PRIMARIES_DISPLAY_P3_ = 2u;
constant uint PRIMARIES_BT2020_ = 3u;
constant uint TRANSFER_SDR = 0u;
constant uint TRANSFER_PQ = 1u;
constant uint TRANSFER_HLG = 2u;
constant float SDR_REFERENCE_WHITE_NITS = 203.0;

float bt709_to_linear(
    float c
) {
    if (c < 0.081) {
        return c / 4.5;
    }
    return metal::pow((c + 0.099) / 1.099, 2.2222223);
}

float pq_to_linear(
    float value
) {
    float v_1 = metal::clamp(value, 0.0, 1.0);
    float v_pow = metal::pow(v_1, 1.0 / 78.84375);
    float numerator = metal::max(v_pow - 0.8359375, 0.0);
    float denominator = metal::max(18.851563 - (18.6875 * v_pow), 0.000001);
    float absolute_nits = 10000.0 * metal::pow(numerator / denominator, 1.0 / 0.15930176);
    return absolute_nits / SDR_REFERENCE_WHITE_NITS;
}

float hlg_to_scene_linear(
    float value_1
) {
    float scene_linear = 0.0;
    float e = metal::clamp(value_1, 0.0, 1.0);
    if (e <= 0.5) {
        scene_linear = (e * e) / 3.0;
    } else {
        scene_linear = (metal::exp((e - 0.5599107) / 0.17883277) + 0.28466892) / 12.0;
    }
    float _e20 = scene_linear;
    return _e20;
}

metal::float3 hlg_scene_to_display_linear(
    metal::float3 scene_rgb
) {
    metal::float3 safe = metal::max(scene_rgb, metal::float3(0.0));
    float scene_luminance = metal::dot(safe, metal::float3(0.2627, 0.678, 0.0593));
    float ootf_gain = metal::pow(metal::max(scene_luminance, 0.000001), 1.2 - 1.0);
    return (safe * ootf_gain) * 4.9261084;
}

metal::float3 decode_transfer_to_linear(
    metal::float3 rgb,
    uint transfer_mode
) {
    if (transfer_mode == TRANSFER_PQ) {
        float _e5 = pq_to_linear(rgb.x);
        float _e7 = pq_to_linear(rgb.y);
        float _e9 = pq_to_linear(rgb.z);
        return metal::float3(_e5, _e7, _e9);
    }
    if (transfer_mode == TRANSFER_HLG) {
        float _e14 = hlg_to_scene_linear(rgb.x);
        float _e16 = hlg_to_scene_linear(rgb.y);
        float _e18 = hlg_to_scene_linear(rgb.z);
        metal::float3 _e20 = hlg_scene_to_display_linear(metal::float3(_e14, _e16, _e18));
        return _e20;
    }
    float _e22 = bt709_to_linear(rgb.x);
    float _e24 = bt709_to_linear(rgb.y);
    float _e26 = bt709_to_linear(rgb.z);
    return metal::float3(_e22, _e24, _e26);
}

float decode_transfer_scalar(
    float value_2,
    uint transfer_mode_1
) {
    if (transfer_mode_1 == TRANSFER_PQ) {
        float _e4 = pq_to_linear(value_2);
        return _e4;
    }
    if (transfer_mode_1 == TRANSFER_HLG) {
        float _e7 = hlg_to_scene_linear(value_2);
        return _e7;
    }
    float _e8 = bt709_to_linear(value_2);
    return _e8;
}

metal::float3 convert_primaries_to_srgb(
    metal::float3 linear_rgb,
    uint primaries_mode
) {
    if (primaries_mode == PRIMARIES_BT2020_) {
        return metal::float3(((1.6605 * linear_rgb.x) - (0.5876 * linear_rgb.y)) - (0.0728 * linear_rgb.z), ((-0.1246 * linear_rgb.x) + (1.1329 * linear_rgb.y)) - (0.0083 * linear_rgb.z), ((-0.0182 * linear_rgb.x) - (0.1006 * linear_rgb.y)) + (1.1188 * linear_rgb.z));
    }
    if (primaries_mode == PRIMARIES_DISPLAY_P3_) {
        return metal::float3(((1.2249 * linear_rgb.x) - (0.2247 * linear_rgb.y)) - (0.0002 * linear_rgb.z), ((-0.042 * linear_rgb.x) + (1.0419 * linear_rgb.y)) + (0.0001 * linear_rgb.z), ((-0.0197 * linear_rgb.x) - (0.0786 * linear_rgb.y)) + (1.0983 * linear_rgb.z));
    }
    return linear_rgb;
}

metal::float3 normalize_yuv(
    float y_sample,
    metal::float2 uv_sample,
    constant ColorParams& color_params
) {
    float y = {};
    float u = {};
    float v = {};
    y = y_sample;
    u = uv_sample.x;
    v = uv_sample.y;
    uint _e9 = color_params.range_mode;
    if (_e9 == RANGE_LIMITED) {
        uint _e14 = color_params.sample_mode;
        if (_e14 == SAMPLE_P010_) {
            float _e17 = y;
            y = (_e17 - 0.062561095) * 1.1678082;
            float _e22 = u;
            u = (_e22 - 0.50048876) * 1.141741;
            float _e27 = v;
            v = (_e27 - 0.50048876) * 1.141741;
        } else {
            float _e32 = y;
            y = (_e32 - 0.0627451) * 1.1643835;
            float _e37 = u;
            u = (_e37 - 0.5019608) * 1.1383928;
            float _e42 = v;
            v = (_e42 - 0.5019608) * 1.1383928;
        }
    } else {
        uint _e49 = color_params.sample_mode;
        if (_e49 == SAMPLE_P010_) {
            float _e52 = u;
            u = _e52 - 0.50048876;
            float _e55 = v;
            v = _e55 - 0.50048876;
        } else {
            float _e58 = u;
            u = _e58 - 0.5019608;
            float _e61 = v;
            v = _e61 - 0.5019608;
        }
    }
    float _e64 = y;
    float _e65 = u;
    float _e66 = v;
    return metal::float3(_e64, _e65, _e66);
}

metal::float3 yuv_to_gamma_rgb(
    metal::float3 yuv,
    constant ColorParams& color_params
) {
    float r = 0.0;
    float g = 0.0;
    float b = 0.0;
    float y_2 = yuv.x;
    float u_1 = yuv.y;
    float v_2 = yuv.z;
    uint _e12 = color_params.matrix_mode;
    if (_e12 == MATRIX_BT601_) {
        r = y_2 + (1.402 * v_2);
        g = (y_2 - (0.344136 * u_1)) - (0.714136 * v_2);
        b = y_2 + (1.772 * u_1);
    } else {
        uint _e29 = color_params.matrix_mode;
        if (_e29 == MATRIX_BT2020_) {
            r = y_2 + (1.4746 * v_2);
            g = (y_2 - (0.164553 * u_1)) - (0.571353 * v_2);
            b = y_2 + (1.8814 * u_1);
        } else {
            r = y_2 + (1.5748 * v_2);
            g = (y_2 - (0.187324 * u_1)) - (0.468124 * v_2);
            b = y_2 + (1.8556 * u_1);
        }
    }
    float _e56 = r;
    float _e57 = g;
    float _e58 = b;
    return metal::max(metal::float3(_e56, _e57, _e58), metal::float3(0.0));
}

metal::float3 bt2020_constant_luminance_to_linear(
    metal::float3 yuv_1,
    constant ColorParams& color_params
) {
    float y_gamma = yuv_1.x;
    float b_gamma = y_gamma + (yuv_1.y * ((yuv_1.y <= 0.0) ? 1.9404 : 1.5816));
    float r_gamma = y_gamma + (yuv_1.z * ((yuv_1.z <= 0.0) ? 1.7184 : 0.9936));
    uint _e22 = color_params.transfer_mode;
    float _e23 = decode_transfer_scalar(y_gamma, _e22);
    uint _e26 = color_params.transfer_mode;
    float _e27 = decode_transfer_scalar(r_gamma, _e26);
    uint _e30 = color_params.transfer_mode;
    float _e31 = decode_transfer_scalar(b_gamma, _e30);
    float g_linear = ((_e23 - (0.2627 * _e27)) - (0.0593 * _e31)) / 0.678;
    metal::float3 linear_rgb_2 = metal::max(metal::float3(_e27, g_linear, _e31), metal::float3(0.0));
    uint _e46 = color_params.transfer_mode;
    if (_e46 == TRANSFER_HLG) {
        metal::float3 _e49 = hlg_scene_to_display_linear(linear_rgb_2);
        return _e49;
    }
    return linear_rgb_2;
}

metal::float3 decode_yuv_to_linear(
    float y_1,
    metal::float2 uv,
    constant ColorParams& color_params
) {
    metal::float3 linear_rgb_1 = metal::float3(0.0);
    metal::float3 _e2 = normalize_yuv(y_1, uv, color_params);
    uint _e8 = color_params.matrix_mode;
    if (_e8 == MATRIX_BT2020_CONSTANT_LUMINANCE) {
        metal::float3 _e11 = bt2020_constant_luminance_to_linear(_e2, color_params);
        linear_rgb_1 = _e11;
    } else {
        metal::float3 _e12 = yuv_to_gamma_rgb(_e2, color_params);
        uint _e15 = color_params.transfer_mode;
        metal::float3 _e16 = decode_transfer_to_linear(_e12, _e15);
        linear_rgb_1 = _e16;
    }
    metal::float3 _e17 = linear_rgb_1;
    uint _e20 = color_params.primaries_mode;
    metal::float3 _e21 = convert_primaries_to_srgb(_e17, _e20);
    return _e21;
}

struct convert_to_linear_rgbaInput {
};
kernel void convert_to_linear_rgba(
  metal::uint3 global_id [[thread_position_in_grid]]
, metal::texture2d<uint, metal::access::sample> y_texture [[texture(0)]]
, metal::texture2d<uint, metal::access::sample> uv_texture [[texture(1)]]
, constant ColorParams& color_params [[buffer(0)]]
, metal::texture2d<float, metal::access::write> linear_rgba_output [[texture(2)]]
) {
    bool local = {};
    metal::uint2 dimensions = metal::uint2(linear_rgba_output.get_width(), linear_rgba_output.get_height());
    if (!((global_id.x >= dimensions.x))) {
        local = global_id.y >= dimensions.y;
    } else {
        local = true;
    }
    bool _e13 = local;
    if (_e13) {
        return;
    }
    metal::int2 y_coordinates = static_cast<metal::int2>(global_id.xy);
    metal::int2 uv_coordinates = metal::int2(static_cast<int>(global_id.x / 2u), static_cast<int>(global_id.y / 2u));
    uint _e27 = color_params.sample_mode;
    float code_scale = (_e27 == SAMPLE_P010_) ? 0.000015259022 : 0.003921569;
    metal::uint4 _e35 = y_texture.read(metal::uint2(y_coordinates), 0);
    float y_3 = static_cast<float>(_e35.x) * code_scale;
    metal::uint4 _e41 = uv_texture.read(metal::uint2(uv_coordinates), 0);
    metal::uint2 uv_raw = _e41.xy;
    metal::float2 uv_1 = metal::float2(static_cast<float>(uv_raw.x), static_cast<float>(uv_raw.y)) * code_scale;
    metal::float3 _e49 = decode_yuv_to_linear(y_3, uv_1, color_params);
    linear_rgba_output.write(metal::float4(metal::max(_e49, metal::float3(0.0)), 1.0), metal::uint2(y_coordinates));
    return;
}
