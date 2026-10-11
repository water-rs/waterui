// Color matrix: a 3x4 matrix over straight-alpha RGB plus a bias column,
// rows in order. On premultiplied colour the bias scales with alpha, so the
// map is linear with an identity alpha row.

struct Params {
    m00: f32,
    m01: f32,
    m02: f32,
    m03: f32,
    m10: f32,
    m11: f32,
    m12: f32,
    m13: f32,
    m20: f32,
    m21: f32,
    m22: f32,
    m23: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let row0 = vec4<f32>(params.m00, params.m01, params.m02, params.m03);
    let row1 = vec4<f32>(params.m10, params.m11, params.m12, params.m13);
    let row2 = vec4<f32>(params.m20, params.m21, params.m22, params.m23);
    return vec4<f32>(dot(row0, color), dot(row1, color), dot(row2, color), color.a);
}
