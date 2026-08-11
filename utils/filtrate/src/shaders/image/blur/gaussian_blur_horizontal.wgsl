// Separable horizontal gaussian blur pass.
//
// Untiled by design: overlapping taps hit the GPU texture cache, which
// benchmarks faster than workgroup shared-memory tiling on tiler GPUs.
// Weights use the incremental gaussian recurrence (GPU Gems 3, ch. 40),
// so the inner loop costs two multiplies per tap instead of an `exp`.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);
    let sigma = max(param(0u), 0.001);
    let radius = max(i32(ceil(sigma * 3.0)), 0);
    if radius == 0 {
        textureStore(output_texture, vec2<i32>(gid.xy), load_input(center));
        return;
    }

    // Incremental gaussian weights, exploiting kernel symmetry:
    // w(o) = exp(-o^2 / (2 sigma^2)); w(o+1) = w(o) * ratio(o);
    // ratio(o) = exp(-(2o + 1) / (2 sigma^2)) advances by a constant factor.
    let inv_two_sigma_sq = 1.0 / (2.0 * sigma * sigma);
    let ratio_step = exp(-2.0 * inv_two_sigma_sq);
    var side_weight = 1.0;
    var side_ratio = exp(-inv_two_sigma_sq);

    var sum = load_input(center);
    var weight_total = 1.0;
    for (var offset = 1; offset <= radius; offset++) {
        side_weight *= side_ratio;
        side_ratio *= ratio_step;
        sum += (load_input(center + vec2<i32>(-offset, 0))
            + load_input(center + vec2<i32>(offset, 0)))
            * side_weight;
        weight_total += 2.0 * side_weight;
    }
    textureStore(output_texture, vec2<i32>(gid.xy), sum / weight_total);
}
