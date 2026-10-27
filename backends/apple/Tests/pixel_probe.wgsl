// The pixel-transition probe: swaps red into blue and dims green, so the
// effect's output is distinct from the content it captures.
@fragment
fn main(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(input_texture, input_sampler, in.uv);
    return vec4<f32>(c.b, c.g * 0.35, c.r * 0.35, c.a);
}
