// A terminal-style CRT post-process: rolling scanlines, a slight barrel
// curvature and a vignette. `effect_param(0)` is the scanline strength.
//
// The ShaderEffect prelude supplies `input_texture`, `input_sampler`,
// `uniforms` (time, resolution, …), `effect_param` and `VertexOutput`.

fn barrel(uv: vec2<f32>) -> vec2<f32> {
    let centered = uv * 2.0 - 1.0;
    let bent = centered * (1.0 + 0.06 * dot(centered, centered));
    return bent * 0.5 + 0.5;
}

@fragment
fn main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = barrel(in.uv);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    let color = textureSample(input_texture, input_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));

    let row = u32(in.position.y) + u32(uniforms.time * 8.0);
    let scan = select(1.0, 0.0, (row / 2u) % 2u == 1u);
    let shade = mix(1.0, scan, effect_param(0u));

    let centered = in.uv - vec2<f32>(0.5);
    let vignette = clamp(1.0 - 1.4 * dot(centered, centered), 0.0, 1.0);

    return select(vec4<f32>(0.0), vec4<f32>(color.rgb * shade * vignette, color.a), inside);
}
