// Specialized fragment body for user backdrop and content effect pipelines:
// those pipelines only ever draw `KIND_SPAN` + `PAINT_BACKDROP` quads.
fn fs_full(fragment: VsOut) -> vec4<f32> {
    let in = device_quad_pixel(fragment);
    let i = in.instance;
    let flags = (in.meta_.w >> 24u) & 0xffu;
    var cov: f32;
    if (flags & FLAG_HAS_CLIP) != 0u {
        let cp = clip_planes(in);
        var planes: JointPlanes;
        for (var k = 0u; k < cp.n; k = k + 1u) {
            planes[k] = cp.p[k];
        }
        cov = planes_coverage(planes, cp.n) * mask_coverage(in);
    } else {
        cov = mask_coverage(in);
    }
    // union-stub
    if (flags & FLAG_UNION) != 0u {
        let base = bitcast<u32>(instances[i].uv.x);
        let ord = bitcast<u32>(instances[i].uv.y);
        member_field = union_field(base, ord, in.device);
        cov *= union_coverage(member_field, instances[i].params.x);
    }
    // union-stub
    cov = clamp(cov, 0.0, 1.0) * in.params.y;
    if (flags & FLAG_BLEND_SRC) != 0u {
        let coord = vec2<i32>(floor(in.device - instances[i].color.yz));
        let cb = read_composite_backdrop(coord);
        if cov <= 0.0 {
            return cb;
        }
        let space = select(SPACE_SRGB, SPACE_LINEAR, globals.space == SPACE_SRGB);
        let cs = move_space(paint_backdrop(i, in.device) * cov, SPACE_LINEAR, space);
        let over = cs + move_space(cb, globals.space, space) * (1.0 - cs.a);
        return move_space(over, space, globals.space);
    }
    return move_space(paint_backdrop(i, in.device) * cov, SPACE_LINEAR, globals.space);
}
