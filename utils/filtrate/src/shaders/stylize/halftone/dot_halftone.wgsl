// Dot halftone: black ink dots on white paper, dot area proportional to
// darkness — the print-screen convention (dark input → large dots).
//
// Parameters: scale (cell size in pixels), angle (degrees), center.x,
// center.y (uv space). Dots are antialiased over a one-pixel band; the dot
// radius reaches sqrt(2)/2 cell units so solid black actually prints solid.

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
    let rot = rotate2(rel, angle);
    let local = fract(rot / scale) - vec2<f32>(0.5);

    let ink = 1.0 - luminance(base.rgb);
    // sqrt(2)/2 covers the cell corners at full ink.
    let radius = 0.70710678 * sqrt(max(ink, 0.0));
    // One-pixel antialiasing band, expressed in cell units.
    let aa = 1.0 / scale;
    let coverage = 1.0 - smoothstep(radius - aa, radius + aa, length(local));
    let value = 1.0 - coverage;
    textureStore(
        output_texture,
        vec2<i32>(gid.xy),
        vec4<f32>(vec3<f32>(value * base.a), base.a),
    );
}
