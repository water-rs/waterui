// 3x3 convolution with a user-supplied kernel.
//
// Parameters: 9 floats stored row-major (top-left to bottom-right).
// The shader does NOT auto-normalise the kernel; callers are responsible
// for choosing weights that sum to 1.0 (or otherwise) per their intent.
// All four channels are convolved together, which is the correct linear
// operation on premultiplied-alpha data; a kernel summing to zero
// therefore also zeroes coverage.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let center = map_to_input(gid.xy);

    var acc = vec4<f32>(0.0);
    var idx: u32 = 0u;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            acc += load_input(center + vec2<i32>(dx, dy)) * param(idx);
            idx = idx + 1u;
        }
    }
    textureStore(output_texture, vec2<i32>(gid.xy), acc);
}
