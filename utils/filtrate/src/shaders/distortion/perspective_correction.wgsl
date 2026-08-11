// Perspective correction: the inverse of perspective transform. The four
// corner points select a quad in the source image (for example a document
// photographed at an angle), and the output unwarps that quad to fill the
// unit square, via a true homography — straight lines stay straight.
//
// Parameters: tl.x, tl.y, tr.x, tr.y, br.x, br.y, bl.x, bl.y — source
// corners in uv space of the quad to rectify.

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

    // H maps the unit square onto the source quad; each output pixel in the
    // unit square samples the source at H(uv) directly.
    let h = unit_square_homography(tl, tr, br, bl);
    let src = apply_homography(h, uv);
    textureStore(output_texture, vec2<i32>(gid.xy), sample_input_bilinear(src));
}
