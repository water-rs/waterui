const START_COLOR: vec3<f32> = vec3<f32>(__START_R__, __START_G__, __START_B__);
const END_COLOR: vec3<f32> = vec3<f32>(__END_R__, __END_G__, __END_B__);
const START_POINT: vec2<f32> = vec2<f32>(__START_X__, __START_Y__);
const END_POINT: vec2<f32> = vec2<f32>(__END_X__, __END_Y__);

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let dir = END_POINT - START_POINT;
    let len_sq = max(dot(dir, dir), 0.0001);
    let rel = uv - START_POINT;
    let t = clamp(dot(rel, dir) / len_sq, 0.0, 1.0);
    return vec4<f32>(mix(START_COLOR, END_COLOR, t), 1.0);
}
