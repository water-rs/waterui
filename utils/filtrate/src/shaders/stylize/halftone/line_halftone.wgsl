// Line halftone: black ink lines on white paper, line width proportional to
// darkness — the print-screen convention (dark input → thick lines).
//
// Parameters: scale (line pitch in pixels), angle (degrees), center.x,
// center.y (uv space). Lines are antialiased over a one-pixel band.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let scale = max(param(0u), 2.0);
    let angle = param(1u) * DEGREES_TO_RADIANS;
    let center = vec2<f32>(param(2u), param(3u));
    let uv = output_uv(gid.xy);
    let base = load_input(map_to_input(gid.xy));

    let rel = (uv - center) * uniforms.output_dimensions;
    let rotated = sin(angle) * rel.x + cos(angle) * rel.y;
    // 0 at the stripe center, 1 halfway to the next stripe.
    let stripe = abs(fract(rotated / scale) - 0.5) * 2.0;

    let ink = 1.0 - luminance(base.rgb);
    // One-pixel antialiasing band, expressed in stripe units.
    let aa = 2.0 / scale;
    let coverage = 1.0 - smoothstep(ink - aa, ink + aa, stripe);
    let value = 1.0 - coverage;
    textureStore(
        output_texture,
        vec2<i32>(gid.xy),
        vec4<f32>(vec3<f32>(value * base.a), base.a),
    );
}
