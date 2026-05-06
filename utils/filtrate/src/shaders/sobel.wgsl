// Sobel edge detection.
//
// Computes the gradient magnitude of the luminance channel via the classic
// Sobel 3x3 kernels, output as a grayscale RGB triple. Alpha is preserved
// from the centre pixel.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

const LUMA: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

fn sample_luma(coord: vec2<i32>, max_xy: vec2<i32>) -> f32 {
    let clamped = clamp(coord, vec2<i32>(0), max_xy);
    let texel = textureLoad(input_texture, clamped, 0);
    return dot(texel.rgb, LUMA);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let in_dims = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let max_xy = in_dims - vec2<i32>(1);

    let tl = sample_luma(coord + vec2<i32>(-1, -1), max_xy);
    let t  = sample_luma(coord + vec2<i32>( 0, -1), max_xy);
    let tr = sample_luma(coord + vec2<i32>( 1, -1), max_xy);
    let l  = sample_luma(coord + vec2<i32>(-1,  0), max_xy);
    let r  = sample_luma(coord + vec2<i32>( 1,  0), max_xy);
    let bl = sample_luma(coord + vec2<i32>(-1,  1), max_xy);
    let b  = sample_luma(coord + vec2<i32>( 0,  1), max_xy);
    let br = sample_luma(coord + vec2<i32>( 1,  1), max_xy);

    let gx = -tl + tr - 2.0 * l + 2.0 * r - bl + br;
    let gy = -tl - 2.0 * t - tr + bl + 2.0 * b + br;
    let magnitude = sqrt(gx * gx + gy * gy);

    let centre = textureLoad(input_texture, clamp(coord, vec2<i32>(0), max_xy), 0);
    textureStore(output_texture, coord, vec4<f32>(magnitude, magnitude, magnitude, centre.a));
}
