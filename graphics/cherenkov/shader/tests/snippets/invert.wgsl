const ONE: f32 = 1.0;

fn apply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(ONE * color.a - color.rgb, color.a);
}
