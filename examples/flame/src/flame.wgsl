// Flame fragment shader for ShaderSurface
// Built-in uniforms are automatically available:
// - uniforms.time: f32
// - uniforms.resolution: vec2<f32>

// Hash function for noise
fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

// 2D noise function
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);

    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));

    let u = f * f * (3.0 - 2.0 * f);

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal Brownian Motion for realistic flames
fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var frequency = 1.0;
    var pos = p;

    for (var i = 0; i < 6; i++) {
        value += amplitude * noise(pos * frequency);
        amplitude *= 0.5;
        frequency *= 2.0;
    }

    return value;
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Flip Y for flame to go upward
    var coord = uv;
    coord.y = 1.0 - coord.y;

    // Center horizontally
    let centered = vec2<f32>((coord.x - 0.5) * 2.0, coord.y);

    // Time-based animation
    let time = uniforms.time;

    // Create flame shape using noise
    var flame_coord = centered;
    flame_coord.y -= time * 0.8;

    // Add turbulence
    let turbulence = fbm(flame_coord * 3.0 + vec2<f32>(time * 0.5, 0.0)) * 0.3;
    flame_coord.x += turbulence;

    // Flame intensity
    let dist_from_center = abs(centered.x);
    let flame_width = 0.4 - coord.y * 0.3;
    let horizontal_fade = smoothstep(flame_width, 0.0, dist_from_center);
    let vertical_fade = pow(1.0 - coord.y, 0.5);
    let noise_val = fbm(flame_coord * 4.0);
    let intensity = horizontal_fade * vertical_fade * (noise_val * 0.5 + 0.5);

    // Color gradient: white -> yellow -> orange -> red -> black
    var color = vec3<f32>(0.0);

    if (intensity > 0.0) {
        let t = clamp(intensity, 0.0, 1.0);

        let inner = vec3<f32>(1.0, 0.95, 0.8);
        let middle = vec3<f32>(1.0, 0.5, 0.0);
        let outer = vec3<f32>(0.8, 0.1, 0.0);
        let edge = vec3<f32>(0.2, 0.0, 0.0);

        if (t > 0.7) {
            color = mix(middle, inner, (t - 0.7) / 0.3);
        } else if (t > 0.4) {
            color = mix(outer, middle, (t - 0.4) / 0.3);
        } else if (t > 0.1) {
            color = mix(edge, outer, (t - 0.1) / 0.3);
        } else {
            color = edge * (t / 0.1);
        }

        color += vec3<f32>(0.1, 0.02, 0.0) * intensity;
    }

    let alpha = clamp(intensity * 2.0, 0.0, 1.0);
    return vec4<f32>(color, alpha);
}
