// Dot halftone: black ink dots on white paper, dot area proportional to
// darkness (dark input, large dots).
//
// Parameters: scale (cell size in pixels), angle (degrees), centre (uv).
// Dots are antialiased over a one-pixel band; the dot radius reaches
// sqrt(2)/2 cell units so solid black prints solid.

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
    let s = sin(angle);
    let c = cos(angle);
    let rot = vec2<f32>(c * rel.x - s * rel.y, s * rel.x + c * rel.y);
    let local = fract(rot / scale) - vec2<f32>(0.5);

    let ink = 1.0 - dot(base.rgb, space.luma);
    // sqrt(2)/2 covers the cell corners at full ink.
    let radius = 0.70710678 * sqrt(max(ink, 0.0));
    // One-pixel antialiasing band, in cell units.
    let aa = 1.0 / scale;
    let coverage = 1.0 - smoothstep(radius - aa, radius + aa, length(local));
    let value = 1.0 - coverage;
    return vec4<f32>(vec3<f32>(value * base.a), base.a);
}
