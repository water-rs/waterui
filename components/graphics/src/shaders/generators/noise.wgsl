const SEED: f32 = __SEED__;

fn hash21(p: vec2<f32>) -> f32 {
    let dotp = dot(p + vec2<f32>(SEED, SEED * 1.37), vec2<f32>(127.1, 311.7));
    return fract(sin(dotp) * 43758.5453123);
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = floor(uv * uniforms.resolution);
    let n = hash21(pixel);
    return vec4<f32>(vec3<f32>(n), 1.0);
}
