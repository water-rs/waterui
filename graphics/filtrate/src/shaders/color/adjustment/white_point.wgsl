// White point: normalizes colour by a source white point, a per-channel
// scale.

struct Params {
    red: f32,
    green: f32,
    blue: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let white_point = max(vec3<f32>(params.red, params.green, params.blue), vec3<f32>(0.0001));
    return vec4<f32>(color.rgb / white_point, color.a);
}
