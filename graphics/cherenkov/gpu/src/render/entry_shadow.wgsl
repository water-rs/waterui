@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return fs_shadow(in);
}
