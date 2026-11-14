@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let left = vec3<f32>(0.95, 0.30, 0.40);
    let right = vec3<f32>(0.20, 0.55, 0.95);
    let bar = step(0.5, fract(uv.x * 6.0));
    let color = mix(left, right, uv.y) * mix(0.6, 1.0, bar);
    return vec4<f32>(color, 1.0);
}
