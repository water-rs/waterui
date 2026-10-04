enable f16;

struct Params {
    amount: f32,
}

fn apply(color: vec4<f16>, params: Params) -> vec4<f16> {
    return vec4<f16>(color.rgb + f16(params.amount) * color.a, color.a);
}
