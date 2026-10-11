struct Params {
    amount: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}

fn apply(color: vec4<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let luma = dot(color.rgb, space.luma);
    return vec4<f32>(mix(vec3<f32>(luma), color.rgb, params.amount), color.a);
}
