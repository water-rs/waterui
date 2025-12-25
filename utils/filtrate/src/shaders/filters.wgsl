// Uber-shader for all filter types
// filter_type: 0=blur, 1=brightness, 2=saturation, 3=contrast, 4=grayscale,
//              5=hue_rotation, 6=invert, 7=sepia, 8=vignette, 9=sharpen

struct FilterUniform {
    filter_type: u32,
    param1: f32,
    param2: f32,
    _padding: u32,
    dimensions: vec2<f32>,
    _padding2: vec2<f32>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: FilterUniform;
@group(0) @binding(3) var texture_sampler: sampler;

// Constants
const PI: f32 = 3.14159265359;

// Helper: Convert RGB to HSL
fn rgb_to_hsl(rgb: vec3<f32>) -> vec3<f32> {
    let max_c = max(max(rgb.r, rgb.g), rgb.b);
    let min_c = min(min(rgb.r, rgb.g), rgb.b);
    let l = (max_c + min_c) / 2.0;

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
    h /= 6.0;

    return vec3<f32>(h, s, l);
}

// Helper: Convert HSL to RGB
fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    if hsl.y == 0.0 {
        return vec3<f32>(hsl.z, hsl.z, hsl.z);
    }

    let q = select(hsl.z + hsl.y - hsl.z * hsl.y, hsl.z * (1.0 + hsl.y), hsl.z < 0.5);
    let p = 2.0 * hsl.z - q;

    return vec3<f32>(
        hue_to_rgb(p, q, hsl.x + 1.0 / 3.0),
        hue_to_rgb(p, q, hsl.x),
        hue_to_rgb(p, q, hsl.x - 1.0 / 3.0)
    );
}

fn hue_to_rgb(p: f32, q: f32, t_in: f32) -> f32 {
    var t = t_in;
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 1.0 / 2.0 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

// Filter implementations

fn apply_brightness(color: vec4<f32>, amount: f32) -> vec4<f32> {
    return vec4<f32>(clamp(color.rgb + amount, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
}

fn apply_saturation(color: vec4<f32>, amount: f32) -> vec4<f32> {
    let luminance = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let gray = vec3<f32>(luminance, luminance, luminance);
    return vec4<f32>(mix(gray, color.rgb, amount), color.a);
}

fn apply_contrast(color: vec4<f32>, amount: f32) -> vec4<f32> {
    let adjusted = (color.rgb - 0.5) * amount + 0.5;
    return vec4<f32>(clamp(adjusted, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
}

fn apply_grayscale(color: vec4<f32>, intensity: f32) -> vec4<f32> {
    let luminance = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let gray = vec3<f32>(luminance, luminance, luminance);
    return vec4<f32>(mix(color.rgb, gray, intensity), color.a);
}

fn apply_hue_rotation(color: vec4<f32>, angle: f32) -> vec4<f32> {
    var hsl = rgb_to_hsl(color.rgb);
    hsl.x = fract(hsl.x + angle / 360.0);
    return vec4<f32>(hsl_to_rgb(hsl), color.a);
}

fn apply_invert(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(1.0 - color.rgb, color.a);
}

fn apply_sepia(color: vec4<f32>, intensity: f32) -> vec4<f32> {
    let sepia = vec3<f32>(
        dot(color.rgb, vec3<f32>(0.393, 0.769, 0.189)),
        dot(color.rgb, vec3<f32>(0.349, 0.686, 0.168)),
        dot(color.rgb, vec3<f32>(0.272, 0.534, 0.131))
    );
    return vec4<f32>(mix(color.rgb, sepia, intensity), color.a);
}

fn apply_vignette(color: vec4<f32>, coord: vec2<f32>, radius: f32, softness: f32) -> vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(coord, center);
    let vignette = smoothstep(radius, radius - softness, dist);
    return vec4<f32>(color.rgb * vignette, color.a);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(params.dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let coord = vec2<i32>(global_id.xy);
    let uv = vec2<f32>(global_id.xy) / params.dimensions;
    var color = textureLoad(input_texture, coord, 0);

    switch params.filter_type {
        // Blur (handled separately for multi-pass)
        case 0u: {
            color = apply_blur(coord, i32(params.param1));
        }
        // Brightness
        case 1u: {
            color = apply_brightness(color, params.param1);
        }
        // Saturation
        case 2u: {
            color = apply_saturation(color, params.param1);
        }
        // Contrast
        case 3u: {
            color = apply_contrast(color, params.param1);
        }
        // Grayscale
        case 4u: {
            color = apply_grayscale(color, params.param1);
        }
        // Hue Rotation
        case 5u: {
            color = apply_hue_rotation(color, params.param1);
        }
        // Invert
        case 6u: {
            color = apply_invert(color);
        }
        // Sepia
        case 7u: {
            color = apply_sepia(color, params.param1);
        }
        // Vignette
        case 8u: {
            color = apply_vignette(color, uv, params.param1, params.param2);
        }
        // Sharpen
        case 9u: {
            color = apply_sharpen(coord, params.param1);
        }
        default: {}
    }

    textureStore(output_texture, coord, color);
}

// Box blur (simple, can be extended to Gaussian)
fn apply_blur(coord: vec2<i32>, radius: i32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    var count = 0.0;

    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let sample_coord = coord + vec2<i32>(x, y);
            if sample_coord.x >= 0 && sample_coord.x < i32(params.dimensions.x) &&
               sample_coord.y >= 0 && sample_coord.y < i32(params.dimensions.y) {
                sum += textureLoad(input_texture, sample_coord, 0);
                count += 1.0;
            }
        }
    }

    return sum / count;
}

// Sharpen using unsharp mask
fn apply_sharpen(coord: vec2<i32>, amount: f32) -> vec4<f32> {
    let center = textureLoad(input_texture, coord, 0);

    // Sample neighbors
    let top = textureLoad(input_texture, coord + vec2<i32>(0, -1), 0);
    let bottom = textureLoad(input_texture, coord + vec2<i32>(0, 1), 0);
    let left = textureLoad(input_texture, coord + vec2<i32>(-1, 0), 0);
    let right = textureLoad(input_texture, coord + vec2<i32>(1, 0), 0);

    // Laplacian kernel
    let laplacian = center * 4.0 - top - bottom - left - right;

    // Add sharpened detail
    let result = center + laplacian * amount;
    return vec4<f32>(clamp(result.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), center.a);
}
