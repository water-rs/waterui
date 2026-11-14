// A 2D rotation in radians, shared by the radial distortions.

fn rotate2(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let s = sin(angle);
    let c = cos(angle);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}
