// Luma curve: moves the luma of straight-alpha sRGB along a tone curve and
// scales the chroma around it.
//
//   Y    = dot(c, luma)
//   f(Y) = (1 - amount) * Y + amount * bezier(Y) + offset
//   out  = f(Y) + chroma * (c - Y)
//
// `bezier` is the cubic Bézier over the four control values, defined on
// [0, 1]; a luma outside it evaluates the curve at the nearer end, while the
// linear term and the chroma stay extended.

struct Params {
    v0: f32,
    v1: f32,
    v2: f32,
    v3: f32,
    amount: f32,
    chroma: f32,
    offset: f32,
    luma_r: f32,
    luma_g: f32,
    luma_b: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let straight = color.rgb / max(color.a, 1e-6);
    let luma = dot(straight, vec3<f32>(params.luma_r, params.luma_g, params.luma_b));
    let t = clamp(luma, 0.0, 1.0);
    let u = 1.0 - t;
    let curve = u * u * u * params.v0
        + 3.0 * t * u * u * params.v1
        + 3.0 * t * t * u * params.v2
        + t * t * t * params.v3;
    let tone = mix(luma, curve, params.amount) + params.offset;
    let adjusted = vec3<f32>(tone) + params.chroma * (straight - vec3<f32>(luma));
    return vec4<f32>(adjusted * color.a, color.a);
}
