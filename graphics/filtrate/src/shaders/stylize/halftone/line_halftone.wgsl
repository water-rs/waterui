// Line halftone: black ink lines on white paper, line width proportional to
// darkness (dark input, thick lines).
//
// Parameters: scale (line pitch in pixels), angle (degrees), centre (uv).
// Lines are antialiased over a one-pixel band.

struct Params {
    scale: f32,
    angle: f32,
    center_x: f32,
    center_y: f32,
}

struct WorkingSpace {
    luma: vec3<f32>,
}

const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;

fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params, space: WorkingSpace) -> vec4<f32> {
    let scale = max(params.scale, 2.0);
    let angle = params.angle * DEGREES_TO_RADIANS;
    let base = textureSampleLevel(input, input_point_sampler, (floor(uv * size) + 0.5) / size, 0.0);

    let rel = (uv - vec2<f32>(params.center_x, params.center_y)) * size;
    let rotated = sin(angle) * rel.x + cos(angle) * rel.y;
    // 0 at the stripe centre, 1 halfway to the next stripe.
    let stripe = abs(fract(rotated / scale) - 0.5) * 2.0;

    let ink = 1.0 - dot(base.rgb, space.luma);
    // One-pixel antialiasing band, in stripe units.
    let aa = 2.0 / scale;
    let coverage = 1.0 - smoothstep(ink - aa, ink + aa, stripe);
    let value = 1.0 - coverage;
    return vec4<f32>(vec3<f32>(value * base.a), base.a);
}
