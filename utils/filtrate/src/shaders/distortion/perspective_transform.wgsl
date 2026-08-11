// Perspective transform: warps the image so the unit square lands on the
// quad given by four corner points, via a true homography (projective
// mapping with the w divide) — straight lines in the source stay straight.
//
// Parameters: tl.x, tl.y, tr.x, tr.y, br.x, br.y, bl.x, bl.y — destination
// corners in uv space of where the image's own corners should land.
// Output pixels outside the destination quad are transparent.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = output_uv(gid.xy);
    let tl = vec2<f32>(param(0u), param(1u));
    let tr = vec2<f32>(param(2u), param(3u));
    let br = vec2<f32>(param(4u), param(5u));
    let bl = vec2<f32>(param(6u), param(7u));

    // H maps the unit square onto the destination quad; the output pixel
    // therefore samples the source at H^-1(uv).
    let h = unit_square_homography(tl, tr, br, bl);
    let src = apply_homography(inverse3x3(h), uv);

    if src.x < 0.0 || src.x > 1.0 || src.y < 0.0 || src.y > 1.0 {
        textureStore(output_texture, vec2<i32>(gid.xy), vec4<f32>(0.0));
        return;
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(src));
}
