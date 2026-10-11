// Invert: `1 - c` on straight-alpha colour, which is `a - rgb` on
// premultiplied colour — linear, with an identity alpha row.

fn apply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.a - color.rgb, color.a);
}
