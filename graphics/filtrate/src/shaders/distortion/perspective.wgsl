// Perspective warp through a true homography (projective mapping with the w
// divide), so straight lines stay straight. The four corners are a quad in
// uv space, and H maps the unit square onto it.
//
// `inverse` selects the direction. Perspective transform (`inverse = 1`)
// lands the image's own corners on the quad: each output pixel samples the
// input at H^-1(uv), and pixels outside the quad are transparent.
// Perspective correction (`inverse = 0`) unwarps the quad to fill the
// output: each output pixel samples the input at H(uv).

struct Params {
    tl_x: f32,
    tl_y: f32,
    tr_x: f32,
    tr_y: f32,
    br_x: f32,
    br_y: f32,
    bl_x: f32,
    bl_y: f32,
    inverse: f32,
}

// Columns of H such that H * (u, v, 1) projects to the quad point for
// square coordinates (u, v).
fn unit_square_homography(tl: vec2<f32>, tr: vec2<f32>, br: vec2<f32>, bl: vec2<f32>) -> mat3x3<f32> {
    let s = tl - tr + br - bl;
    let d1 = tr - br;
    let d2 = bl - br;
    let denom = d1.x * d2.y - d1.y * d2.x;
    var g = 0.0;
    var h = 0.0;
    if abs(s.x) > 1e-6 || abs(s.y) > 1e-6 {
        g = (s.x * d2.y - s.y * d2.x) / denom;
        h = (d1.x * s.y - d1.y * s.x) / denom;
    }
    let a = tr.x - tl.x + g * tr.x;
    let b = bl.x - tl.x + h * bl.x;
    let c = tl.x;
    let d = tr.y - tl.y + g * tr.y;
    let e = bl.y - tl.y + h * bl.y;
    let f = tl.y;
    return mat3x3<f32>(
        vec3<f32>(a, d, g),
        vec3<f32>(b, e, h),
        vec3<f32>(c, f, 1.0),
    );
}

fn apply_homography(m: mat3x3<f32>, p: vec2<f32>) -> vec2<f32> {
    let q = m * vec3<f32>(p, 1.0);
    return q.xy / q.z;
}

// The inverse through the adjugate: the cross products are the rows of the
// inverse, transposed into columns.
fn inverse3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    let det = dot(m[0], c0);
    return transpose(mat3x3<f32>(c0, c1, c2)) * (1.0 / det);
}

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let h = unit_square_homography(
        vec2<f32>(params.tl_x, params.tl_y),
        vec2<f32>(params.tr_x, params.tr_y),
        vec2<f32>(params.br_x, params.br_y),
        vec2<f32>(params.bl_x, params.bl_y),
    );
    if params.inverse < 0.5 {
        return textureSampleLevel(input, input_sampler, apply_homography(h, uv), 0.0);
    }
    let src = apply_homography(inverse3x3(h), uv);
    if src.x < 0.0 || src.x > 1.0 || src.y < 0.0 || src.y > 1.0 {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(input, input_sampler, src, 0.0);
}
