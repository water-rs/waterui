// Motion blur: a directional blur along `angle` (degrees), `radius` pixels
// each way, with a triangular weight. Taps filter bilinearly, so off-axis
// directions produce a smooth streak instead of duplicated nearest texels.

struct Params {
    radius: f32,
    angle: f32,
}

const DEGREES_TO_RADIANS: f32 = 0.017453292519943295;

fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> {
    let radius = max(i32(round(params.radius)), 0);
    if radius == 0 {
        return textureSampleLevel(input, input_sampler, (floor(uv * size) + 0.5) / size, 0.0);
    }

    let mapped = uv * size;
    let angle = params.angle * DEGREES_TO_RADIANS;
    let direction = vec2<f32>(cos(angle), sin(angle));

    var sum = vec4<f32>(0.0);
    var total_weight = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let sample_uv = (mapped + direction * f32(i)) / size;
        let weight = 1.0 - abs(f32(i)) / f32(radius + 1);
        sum += textureSampleLevel(input, input_sampler, sample_uv, 0.0) * weight;
        total_weight += weight;
    }
    return sum / total_weight;
}
