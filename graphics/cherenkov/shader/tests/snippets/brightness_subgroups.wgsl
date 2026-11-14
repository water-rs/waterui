struct Params {
    amount: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let alpha = subgroupMax(color.a);
    return vec4<f32>(color.rgb + params.amount * alpha, color.a);
}
