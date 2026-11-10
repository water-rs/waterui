// Solid atlas cells and full spans under a sharp rectangular clip.
fn fs_full(fragment: VsOut) -> vec4<f32> {
    let in = device_quad_pixel(fragment);
    let i = in.instance;
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov = 1.0;
    if in.meta_.x == KIND_GLYPH {
        cov = glyph_coverage(in);
    }
    if (flags & FLAG_HAS_CLIP) != 0u {
        let pc = apply(instances[i].clip_inv, in.device);
        let sample = rect_sample(instances[i].clip.half, pc);
        if in.meta_.x == KIND_SPAN {
            let cp = clip_sample_planes(in, pc, sample, true);
            var planes: JointPlanes;
            for (var k = 0u; k < cp.n; k = k + 1u) {
                planes[k] = cp.p[k];
            }
            cov = planes_coverage(planes, cp.n);
        } else {
            cov *= clip_sample_coverage(in, sample);
        }
    }
    cov = clamp(cov * mask_coverage(in), 0.0, 1.0) * in.params.y;
    return move_space(vec4<f32>(in.color * ColorScalar(cov)), SPACE_LINEAR, globals.space);
}
