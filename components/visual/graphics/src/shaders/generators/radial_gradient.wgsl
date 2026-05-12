const INNER_COLOR: vec3<f32> = vec3<f32>(__INNER_R__, __INNER_G__, __INNER_B__);
const OUTER_COLOR: vec3<f32> = vec3<f32>(__OUTER_R__, __OUTER_G__, __OUTER_B__);
const CENTER: vec2<f32> = vec2<f32>(__CENTER_X__, __CENTER_Y__);
const RADIUS: f32 = __RADIUS__;

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let t = clamp(distance(uv, CENTER) / max(RADIUS, 0.0001), 0.0, 1.0);
    return vec4<f32>(mix(INNER_COLOR, OUTER_COLOR, t), 1.0);
}
