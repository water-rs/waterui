// Pixellate: snaps sampling to the center of square cells.
//
// Parameters: cell size in output pixels. Nearest-texel loads are
// intentional — pixellate wants hard-edged cells, not interpolation.

@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(uniforms.output_dimensions);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let size = max(param(0u), 1.0);
    let pixel = vec2<f32>(gid.xy) + vec2<f32>(0.5);
    let cell_center = floor(pixel / size) * size + vec2<f32>(size * 0.5);
    let sample_uv = cell_center / uniforms.output_dimensions;
    let coord = vec2<i32>(sample_uv * uniforms.input_dimensions);
    textureStore(output_texture, vec2<i32>(gid.xy), load_input(coord));
}
