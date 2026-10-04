// The vertex stage of every executor pass: one triangle covering the whole
// target. Fragments read their own pixel from @builtin(position).

@vertex
fn main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}
