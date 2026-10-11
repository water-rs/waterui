// Contrast: scales each straight-alpha channel about 0.5,
// `(c - 0.5) * amount + 0.5`. On premultiplied colour the pivot scales with
// alpha, so the map stays linear with an identity alpha row.

struct Params {
    amount: f32,
}

fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
    let pivot = 0.5 * color.a;
    return vec4<f32>((color.rgb - pivot) * params.amount + pivot, color.a);
}
