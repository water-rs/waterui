struct Params {
    amount: f32,
    pivot: vec3<f32>,
}

fn scale(value: vec3<f32>, pivot: vec3<f32>, amount: f32) -> vec3<f32> {
    return (value - pivot) * amount + pivot;
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    return vec4<f32>(scale(color.rgb, params.pivot * color.a, params.amount), color.a);
}
