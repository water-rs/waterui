// Overscroll stretch: Android's edge-weighted stretch warp, HWUI's
// `stretchShader` (frameworks/base libs/hwui/effects/StretchEffect.cpp),
// remapping each axis of a frame independently.
//
// The frame is an affine map from the stretched frame's own space —
// `[0, width] × [0, height]`, the space HWUI evaluates the shader in — to
// input pixels: `pixel = (a·x + c·y + e, b·x + d·y + f)`. Inside the frame
// each output position samples the input at the position
// `computeOverscroll` maps it to; outside it the input passes through.
//
// `stretch_x`/`stretch_y` are HWUI's merged stretch direction: positive
// pulls the left/top edge, negative the right/bottom one. The interpolation
// strength is HWUI's `INTERPOLATION_STRENGTH_VALUE`, specialized as a
// constant.

struct Params {
    frame_a: f32,
    frame_b: f32,
    frame_c: f32,
    frame_d: f32,
    frame_e: f32,
    frame_f: f32,
    width: f32,
    height: f32,
    stretch_x: f32,
    stretch_y: f32,
    interpolation_strength: f32,
}

// HWUI's `computeOverscroll` with `uStretchAffectedDist = extent` and
// `uInverseStretchAffectedDist = 1 / extent`, as `StretchEffect::getShader`
// sets them, for a position inside `[0, extent]`: `computeOverscrollStart`
// for a positive overscroll, `computeOverscrollEnd` for a negative one.
fn stretch_axis(position: f32, overscroll: f32, extent: f32, strength: f32) -> f32 {
    let stretched = extent / (1.0 + abs(overscroll));
    let inverse = 1.0 / extent;
    var source = position;
    if overscroll > 0.0 {
        let offset = extent - position;
        let variation = mix(1.0, offset * inverse, strength);
        source = stretched - offset / (1.0 + overscroll * variation);
    } else if overscroll < 0.0 {
        let offset = position;
        let variation = mix(1.0, offset * inverse, strength);
        source = extent - (stretched - offset / (1.0 - overscroll * variation));
    }
    return source;
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let relative = uv * size - vec2<f32>(params.frame_e, params.frame_f);
    let determinant = params.frame_a * params.frame_d - params.frame_b * params.frame_c;
    let local = vec2<f32>(
        params.frame_d * relative.x - params.frame_c * relative.y,
        params.frame_a * relative.y - params.frame_b * relative.x,
    ) / determinant;
    var sample_uv = uv;
    if local.x >= 0.0 && local.x <= params.width && local.y >= 0.0 && local.y <= params.height {
        // HWUI samples a snapshot of the node clamped to its edge texels:
        // keep the sample half a pixel inside the frame, so the pulled edge
        // repeats the edge row instead of blending what lies outside.
        let inset = 0.5 / vec2<f32>(
            length(vec2<f32>(params.frame_a, params.frame_b)),
            length(vec2<f32>(params.frame_c, params.frame_d)),
        );
        let extent = vec2<f32>(params.width, params.height);
        let warped = min(max(vec2<f32>(
            stretch_axis(local.x, params.stretch_x, params.width, params.interpolation_strength),
            stretch_axis(local.y, params.stretch_y, params.height, params.interpolation_strength),
        ), inset), extent - inset);
        let source = vec2<f32>(
            params.frame_a * warped.x + params.frame_c * warped.y + params.frame_e,
            params.frame_b * warped.x + params.frame_d * warped.y + params.frame_f,
        );
        sample_uv = source / size;
    }
    return textureSampleLevel(input, input_sampler, sample_uv, 0.0);
}
