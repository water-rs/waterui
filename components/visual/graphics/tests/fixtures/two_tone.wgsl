@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let top = vec3<f32>(0.07, 0.09, 0.16);
    let bottom = vec3<f32>(0.22, 0.74, 0.97);
    let color = mix(top, bottom, uv.y);
    return vec4<f32>(color, 1.0);
}
