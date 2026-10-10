// media.md's `.effect(..)` snippet includes this file by name; it is a real
// shader effect — scanlines with a faint flicker — so `ShaderEffect::new`
// validates it.
@fragment
fn main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(input_texture, input_sampler, in.uv);
    let row = u32(in.position.y);
    let line = select(1.0, 0.0, (row / 2u) % 2u == 1u);
    let flicker = 0.97 + 0.03 * sin(uniforms.time * 50.0);
    let shade = mix(1.0, line, effect_param(0u)) * flicker;
    return vec4<f32>(color.rgb * shade, color.a);
}
