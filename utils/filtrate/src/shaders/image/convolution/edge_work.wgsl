// Edge work: Sobel gradient magnitude at a configurable sampling radius,
// scaled by a strength parameter.
//
// Parameters: radius (pixels), amount. Output is a grayscale RGB triple
// scaled by the centre pixel's alpha so premultiplied coverage stays valid.

fn sample_luma(coord: vec2<i32>) -> f32 {
    return luminance(load_input(coord).rgb);
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let radius = max(i32(round(param(0u))), 1);
    let amount = max(param(1u), 0.0);
    let center = map_to_input(gid.xy);

    let tl = sample_luma(center + vec2<i32>(-radius, -radius));
    let tc = sample_luma(center + vec2<i32>(0, -radius));
    let tr = sample_luma(center + vec2<i32>(radius, -radius));
    let ml = sample_luma(center + vec2<i32>(-radius, 0));
    let mr = sample_luma(center + vec2<i32>(radius, 0));
    let bl = sample_luma(center + vec2<i32>(-radius, radius));
    let bc = sample_luma(center + vec2<i32>(0, radius));
    let br = sample_luma(center + vec2<i32>(radius, radius));

    let gx = -tl - 2.0 * ml - bl + tr + 2.0 * mr + br;
    let gy = -tl - 2.0 * tc - tr + bl + 2.0 * bc + br;
    let edge = clamp(length(vec2<f32>(gx, gy)) * amount, 0.0, 1.0);

    let centre_alpha = load_input(center).a;
    textureStore(
        output_texture,
        vec2<i32>(gid.xy),
        vec4<f32>(vec3<f32>(edge * centre_alpha), centre_alpha),
    );
}
