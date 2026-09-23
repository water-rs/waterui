const PRIMARY: vec3<f32> = vec3<f32>(__PRIMARY_R__, __PRIMARY_G__, __PRIMARY_B__);
const SECONDARY: vec3<f32> = vec3<f32>(__SECONDARY_R__, __SECONDARY_G__, __SECONDARY_B__);
const STRIPE_WIDTH: f32 = __STRIPE_WIDTH__;
const HORIZONTAL: bool = __HORIZONTAL__;
const OFFSET: f32 = __OFFSET__;

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = uv * uniforms.resolution;
    let axis = select(pixel.x, pixel.y, HORIZONTAL) + OFFSET;
    let stripe = i32(floor(axis / STRIPE_WIDTH)) % 2;
    let color = select(SECONDARY, PRIMARY, stripe == 0);
    return vec4<f32>(color, 1.0);
}
