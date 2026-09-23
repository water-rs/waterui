const LIGHT: vec3<f32> = vec3<f32>(__LIGHT_R__, __LIGHT_G__, __LIGHT_B__);
const DARK: vec3<f32> = vec3<f32>(__DARK_R__, __DARK_G__, __DARK_B__);
const CELL_SIZE: vec2<f32> = vec2<f32>(__CELL_W__, __CELL_H__);
const OFFSET: vec2<f32> = vec2<f32>(__OFFSET_X__, __OFFSET_Y__);

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = uv * uniforms.resolution + OFFSET;
    let cell = floor(pixel / CELL_SIZE);
    let parity = i32(cell.x + cell.y) % 2;
    let color = select(DARK, LIGHT, parity == 0);
    return vec4<f32>(color, 1.0);
}
