// Sobel edge detection.
//
// Computes the gradient magnitude of the luminance channel via the classic
// Sobel 3x3 kernels, output as a grayscale RGB triple scaled by the centre
// pixel's alpha so premultiplied coverage stays valid.

fn sample_luma(coord: vec2<i32>) -> f32 {
    return luminance(load_input(coord).rgb);
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);

    let tl = sample_luma(center + vec2<i32>(-1, -1));
    let t  = sample_luma(center + vec2<i32>( 0, -1));
    let tr = sample_luma(center + vec2<i32>( 1, -1));
    let l  = sample_luma(center + vec2<i32>(-1,  0));
    let r  = sample_luma(center + vec2<i32>( 1,  0));
    let bl = sample_luma(center + vec2<i32>(-1,  1));
    let b  = sample_luma(center + vec2<i32>( 0,  1));
    let br = sample_luma(center + vec2<i32>( 1,  1));

    let gx = -tl + tr - 2.0 * l + 2.0 * r - bl + br;
    let gy = -tl - 2.0 * t - tr + bl + 2.0 * b + br;
    let magnitude = clamp(sqrt(gx * gx + gy * gy), 0.0, 1.0);

    let centre = load_input(center);
    textureStore(
        output_texture,
        vec2<i32>(gid.xy),
        vec4<f32>(vec3<f32>(magnitude * centre.a), centre.a),
    );
}
