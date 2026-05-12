const FOREGROUND: vec3<f32> = vec3<f32>(__FG_R__, __FG_G__, __FG_B__);
const BACKGROUND: vec3<f32> = vec3<f32>(__BG_R__, __BG_G__, __BG_B__);
const SPACING: f32 = __SPACING__;
const RADIUS: f32 = __RADIUS__;

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = uv * uniforms.resolution;
    let centered = fract((pixel + SPACING * 0.5) / SPACING) - vec2<f32>(0.5, 0.5);
    let local = centered * SPACING;
    let distance = length(local);
    let aa = max(fwidth(distance), 0.75);
    let coverage = 1.0 - smoothstep(RADIUS - aa, RADIUS + aa, distance);
    let color = mix(BACKGROUND, FOREGROUND, coverage);
    return vec4<f32>(color, 1.0);
}
