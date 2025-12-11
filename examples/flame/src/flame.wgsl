// Test: show time value directly as color
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Red channel = fract(time), should cycle 0->1 every second
    let t = fract(uniforms.time);
    return vec4<f32>(t, 0.0, 0.0, 1.0);
}
