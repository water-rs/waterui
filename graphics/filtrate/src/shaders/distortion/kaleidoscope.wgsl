// Kaleidoscope: folds the image into `segments` mirrored angular slices
// around a centre point.
//
// Parameters: segments, rotation (degrees), centre (uv). Angles are measured
// in isotropic space so slices keep equal angular width on non-square
// images.

struct Params {
    segments: f32,
    rotation: f32,
    center_x: f32,
    center_y: f32,
}

const TAU: f32 = 6.283185307179586;
const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let isotropic = size / min(size.x, size.y);
    let segments = max(params.segments, 2.0);
    let rotation = params.rotation * DEGREES_TO_RADIANS;

    let center = vec2<f32>(params.center_x, params.center_y) * isotropic;
    let delta = uv * isotropic - center;
    let r = length(delta);
    var angle = atan2(delta.y, delta.x) - rotation;
    let slice = TAU / segments;
    angle = abs(fract(angle / slice) - 0.5) * slice;
    let folded = vec2<f32>(cos(angle + rotation), sin(angle + rotation)) * r;
    return textureSampleLevel(input, input_sampler, (center + folded) / isotropic, 0.0);
}
