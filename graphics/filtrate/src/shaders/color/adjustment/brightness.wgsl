// Brightness: adds `amount` to each straight-alpha channel. On premultiplied
// colour that is `rgb + amount * a`, a linear map with an identity alpha row.

struct Params {
    amount: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    return vec4<f32>(color.rgb + params.amount * color.a, color.a);
}
