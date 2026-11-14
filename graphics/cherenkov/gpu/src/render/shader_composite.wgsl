// Unclipped source-over composition; the source retains its recorded space.
fn fs_full(fragment: VsOut) -> vec4<f32> {
    let in = device_quad_pixel(fragment);
    return plain_composite(in, in.params.y);
}
