// Exposure: scales colour by 2^ev (photographic stops).

struct Params {
    ev: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    return vec4<f32>(color.rgb * exp2(params.ev), color.a);
}
