// Separable horizontal gaussian blur pass.
//
// Untiled by design: overlapping taps hit the GPU texture cache, which
// benchmarks faster than workgroup shared-memory tiling on tiler GPUs.
// Weights use the incremental gaussian recurrence (GPU Gems 3, ch. 40),
// so the inner loop costs two multiplies per tap instead of an `exp`.

struct Uniforms {
    output_dimensions: vec2<f32>,
    input_dimensions: vec2<f32>,
    params: array<vec4<f32>, 16>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;


fn param(index: u32) -> f32 {
    let vec_idx = index / 4u;
    let component = index % 4u;
    let v = uniforms.params[vec_idx];
    switch component {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if global_id.x >= dims.x || global_id.y >= dims.y {
        return;
    }

    let input_dims_i = vec2<i32>(vec2<u32>(uniforms.input_dimensions));
    let output_coord = vec2<i32>(global_id.xy);

    let mapped = (vec2<f32>(global_id.xy) + vec2<f32>(0.5)) * uniforms.input_dimensions
        / uniforms.output_dimensions;
    let center = clamp(vec2<i32>(mapped), vec2<i32>(0), input_dims_i - vec2<i32>(1));

    let sigma = max(param(0u), 0.001);
    let radius = max(i32(ceil(sigma * 3.0)), 0);
    if radius == 0 {
        textureStore(output_texture, output_coord, textureLoad(input_texture, center, 0));
        return;
    }

    // Incremental gaussian weights, exploiting kernel symmetry:
    // w(o) = exp(-o^2 / (2 sigma^2)); w(o+1) = w(o) * ratio(o);
    // ratio(o) = exp(-(2o + 1) / (2 sigma^2)) advances by a constant factor.
    let inv_two_sigma_sq = 1.0 / (2.0 * sigma * sigma);
    let ratio_step = exp(-2.0 * inv_two_sigma_sq);
    var side_weight = 1.0;
    var side_ratio = exp(-inv_two_sigma_sq);

    var sum = textureLoad(input_texture, center, 0);
    var weight_total = 1.0;
    for (var offset = 1; offset <= radius; offset++) {
        side_weight *= side_ratio;
        side_ratio *= ratio_step;
        let left = clamp(
            center + vec2<i32>(-offset, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        let right = clamp(
            center + vec2<i32>(offset, 0),
            vec2<i32>(0),
            input_dims_i - vec2<i32>(1),
        );
        sum += (textureLoad(input_texture, left, 0) + textureLoad(input_texture, right, 0))
            * side_weight;
        weight_total += 2.0 * side_weight;
    }

    textureStore(output_texture, output_coord, sum / weight_total);
}
